// Measurement harness only (not product code): a counting global allocator so that
// per-phase heap use can be reported without OS-level RSS noise.
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
fn cur() -> usize {
    CUR.load(Relaxed)
}
fn reset_peak() -> usize {
    let c = cur();
    PEAK.store(c, Relaxed);
    c
}
fn peak() -> usize {
    PEAK.load(Relaxed)
}
fn mb(b: usize) -> f64 {
    b as f64 / 1048576.0
}

fn hayro(data: Vec<u8>, cached: bool) -> (usize, usize, usize, usize, usize, usize) {
    use hayro_syntax::object::{Array, Dict, Stream};
    use hayro_syntax::Pdf;
    let base = reset_peak();
    let pdf = Pdf::new(data).expect("hayro load");
    let load_peak = peak() - base;
    let after_load = cur() - base;
    let mut page_peak_max = 0usize;
    let mut n = 0usize;
    let mut font_bytes = 0usize;
    for page in pdf.pages().iter() {
        n += 1;
        let p0 = reset_peak();
        // content
        if cached {
            let _ = page.page_stream().map(|s| s.len());
        } else {
            let d: &Dict = page.raw();
            if let Some(s) = d.get::<Stream>(b"Contents") {
                let _ = s.decoded().map(|c| c.len());
            } else if let Some(a) = d.get::<Array>(b"Contents") {
                for s in a.iter::<Stream>() {
                    let _ = s.decoded().map(|c| c.len());
                }
            }
        }
        // fonts: decode embedded font programs
        let fonts = page.resources().fonts.clone();
        for name in fonts.keys() {
            let Some(fd) = fonts.get::<Dict>(name.as_ref()) else {
                continue;
            };
            // Type0 -> DescendantFonts[0]
            let desc_font: Dict = fd
                .get::<Array>(b"DescendantFonts")
                .and_then(|a| a.iter::<Dict>().next())
                .unwrap_or(fd.clone());
            if let Some(fdesc) = desc_font.get::<Dict>(b"FontDescriptor") {
                for key in [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"] {
                    if let Some(s) = fdesc.get::<Stream>(key) {
                        if let Ok(c) = s.decoded() {
                            font_bytes += c.len();
                        }
                    }
                }
            }
        }
        page_peak_max = page_peak_max.max(peak() - p0);
    }
    let after_all = cur() - base;
    (
        n,
        load_peak,
        after_load,
        page_peak_max,
        after_all,
        font_bytes,
    )
}

fn lopdf_run(data: &[u8]) -> (usize, usize, usize, usize, usize, usize) {
    use lopdf::{Document, Object};
    let base = reset_peak();
    let doc = Document::load_mem(data).expect("lopdf load");
    let load_peak = peak() - base;
    let after_load = cur() - base;
    let mut page_peak_max = 0usize;
    let mut n = 0usize;
    let mut font_bytes = 0usize;
    for (_num, pid) in doc.get_pages() {
        n += 1;
        let p0 = reset_peak();
        let _ = doc.get_page_content(pid).len();
        if let Ok(fonts) = doc.get_page_fonts(pid) {
            for (_k, fdict) in fonts {
                let desc = fdict
                    .get(b"DescendantFonts")
                    .ok()
                    .and_then(|o| doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_array().ok().and_then(|a| a.first().cloned()))
                    .and_then(|o| doc.dereference(&o).ok().map(|(_, o)| o.clone()));
                let fd: lopdf::Dictionary = match desc {
                    Some(Object::Dictionary(d)) => d,
                    _ => fdict.clone(),
                };
                if let Ok(fdesc) = fd
                    .get(b"FontDescriptor")
                    .and_then(|o| doc.dereference(o))
                    .map(|(_, o)| o.clone())
                {
                    if let Ok(fdesc) = fdesc.as_dict() {
                        for key in [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"] {
                            if let Ok(s) = fdesc.get(key).and_then(|o| doc.dereference(o)) {
                                if let Ok(st) = s.1.as_stream() {
                                    if let Ok(c) = st.decompressed_content() {
                                        font_bytes += c.len();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        page_peak_max = page_peak_max.max(peak() - p0);
    }
    let after_all = cur() - base;
    (
        n,
        load_peak,
        after_load,
        page_peak_max,
        after_all,
        font_bytes,
    )
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("mode: hayro|hayro-cached|lopdf");
    let path = args.next().expect("pdf path");
    let data = std::fs::read(&path).expect("read");
    let size = data.len();
    let r = match mode.as_str() {
        "hayro" => hayro(data, false),
        "hayro-cached" => hayro(data, true),
        "lopdf" => lopdf_run(&data),
        _ => panic!("mode"),
    };
    let (n, load_peak, after_load, page_peak_max, after_all, font_bytes) = r;
    println!(
        "{:<13} {:<24} {:>8} {:>5} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>8.2}",
        mode,
        std::path::Path::new(&path)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        size,
        n,
        mb(load_peak),
        mb(after_load),
        mb(page_peak_max),
        mb(after_all),
        mb(font_bytes)
    );
}
