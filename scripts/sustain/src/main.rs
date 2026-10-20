// Sustained-extraction heap measurement (harness only; counting GlobalAlloc is unsafe and not product code).
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
struct Counting;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let c = CUR.fetch_add(l.size(), Relaxed) + l.size();
            PEAK.fetch_max(c, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        CUR.fetch_sub(l.size(), Relaxed);
        System.dealloc(p, l)
    }
}
#[global_allocator]
static A: Counting = Counting;
fn mb(b: usize) -> f64 {
    b as f64 / 1048576.0
}

fn run_tpe(path: &str) -> (usize, usize) {
    let job = tpe::schema::Job {
        path: path.to_string(),
        backend: "lopdf".into(),
        pages: None,
        password: None,
        max_bytes: None,
        figures_dir: None,
    };
    let r = tpe::pipeline::run_job(&job).expect("run_job");
    (r.pages.len(), r.pages.iter().map(|p| p.text.len()).sum())
}
fn run_lopdf(bytes: &[u8]) -> (usize, usize) {
    let doc = lopdf::Document::load_mem(bytes).expect("lopdf");
    let mut n = 0;
    let mut total = 0;
    for (_, pid) in doc.get_pages() {
        n += 1;
        total += doc.get_page_content(pid).len();
    }
    (n, total)
}
fn run_hayro(bytes: &[u8]) -> (usize, usize) {
    use hayro_syntax::object::{Array, Dict, Stream};
    let pdf = hayro_syntax::Pdf::new(bytes.to_vec()).expect("hayro");
    let mut n = 0;
    let mut total = 0;
    for page in pdf.pages().iter() {
        n += 1;
        let d: &Dict = page.raw();
        if let Some(s) = d.get::<Stream>(b"Contents") {
            if let Ok(c) = s.decoded() {
                total += c.len();
            }
        } else if let Some(a) = d.get::<Array>(b"Contents") {
            for s in a.iter::<Stream>() {
                if let Ok(c) = s.decoded() {
                    total += c.len();
                }
            }
        }
    }
    (n, total)
}

fn main() {
    let mut a = std::env::args().skip(1);
    let mode = a.next().expect("mode tpe|lopdf|hayro");
    let iters: usize = a.next().expect("iterations").parse().unwrap();
    let files: Vec<String> = a.collect();
    if mode == "tpe" {
        tpe::pipeline::warm_up();
    }
    let inputs: Vec<Vec<u8>> = files.iter().map(|f| std::fs::read(f).unwrap()).collect();
    let base = CUR.load(Relaxed);
    PEAK.store(base, Relaxed);
    println!(
        "mode={mode} files={} iterations={iters} baseline_heap={:.2}MiB",
        files.len(),
        mb(base)
    );
    println!(
        "{:>4} {:>3} {:<22} {:>6} {:>10} {:>12} {:>12}",
        "iter", "doc", "file", "pages", "text_KB", "retained_MiB", "peak_MiB"
    );
    let mut first_iter_retained = 0usize;
    for it in 1..=iters {
        for (i, f) in files.iter().enumerate() {
            let (n, t) = match mode.as_str() {
                "tpe" => run_tpe(f),
                "lopdf" => run_lopdf(&inputs[i]),
                "hayro" => run_hayro(&inputs[i]),
                _ => panic!(),
            };
            let retained = CUR.load(Relaxed).saturating_sub(base);
            println!(
                "{:>4} {:>3} {:<22} {:>6} {:>10} {:>12.3} {:>12.2}",
                it,
                i + 1,
                std::path::Path::new(f)
                    .file_name()
                    .unwrap()
                    .to_string_lossy(),
                n,
                t / 1024,
                mb(retained),
                mb(PEAK.load(Relaxed) - base)
            );
        }
        let r = CUR.load(Relaxed).saturating_sub(base);
        if it == 1 {
            first_iter_retained = r;
        }
        if it == iters {
            println!("retained after iteration 1 = {:.3} MiB; after iteration {} = {:.3} MiB; delta per iteration = {:.4} MiB; peak = {:.2} MiB", mb(first_iter_retained), it, mb(r), if iters > 1 { mb(r.saturating_sub(first_iter_retained)) / (iters - 1) as f64 } else { 0.0 }, mb(PEAK.load(Relaxed) - base));
        }
    }
}
