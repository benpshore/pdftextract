import * as T from '../lib/folder-traversal.ts';

// Shared by the Node CI suite and the real-browser runner. All bytes are synthetic.
export async function runFolderRegressions() {
  const outcomes = [];
  const equal = (actual, expected, message) => {
    if (actual !== expected) throw new Error(`${message}: ${actual} !== ${expected}`);
  };
  const test = async (name, run) => {
    try { await run(); outcomes.push({ name, ok: true }); }
    catch (error) { outcomes.push({ name, ok: false, error: String(error) }); }
  };
  const collect = files => T.collectFolder('fixture', { candidates: files.map(file => ({
    path: `fixture/${file.name}`, name: file.name, open: async () => file,
  })) });
  await test('sample collisions retain distinct files; full duplicates are skipped', async () => {
    const block = new Blob([new Uint8Array(1024 * 1024)]);
    const a = new File(Array(65).fill(block), 'a.pdf');
    const middle = 32 * 1024 * 1024;
    const b = new File([a.slice(0, middle), new Uint8Array([1]), a.slice(middle + 1)], 'b.pdf');
    const twin = new File([a], 'copy.pdf');
    for (const file of [a, b, twin]) {
      file.arrayBuffer = async () => { throw new Error('Large file must never be copied whole'); };
      const slice = file.slice.bind(file);
      file.slice = (start, end) => {
        if (end - start > 4 * 1024 * 1024) throw new Error('Unbounded slice read');
        return slice(start, end);
      };
    }
    equal(await T.digestFile(a), await T.digestFile(b), 'fixture must collide under sample hashing');
    const result = await collect([a, b, twin]);
    equal(result.files.length, 2, 'distinct originals retained');
    equal(result.files[0].file, a, 'first original reference unchanged');
    equal(result.files[1].file, b, 'second original reference unchanged');
    equal(result.skipped.length, 1, 'only true duplicate skipped');
    equal(result.skipped[0].path, 'fixture/copy.pdf', 'true duplicate identity');
    equal(new Uint8Array(await b.slice(middle, middle + 1).arrayBuffer())[0], 1, 'original byte unchanged');
  });
  await test('equal relative paths do not prove equal content', async () => {
    const result = await collect([new File(['first'], 'same.txt'), new File(['other'], 'same.txt')]);
    equal(result.files.length, 2, 'both same-path originals retained');
  });
  await test('directory handles stop pulling at the budget', async () => {
    let pulls = 0, closed = false;
    const root = { kind: 'directory', name: 'fixture', async *values() {
      try { for (let i = 0; i < 100; i++) { pulls++; yield { kind: 'file', name: `${i}.txt`, getFile: async () => new File(['x'], `${i}.txt`) }; } }
      finally { closed = true; }
    } };
    const scan = await T.enumerateDirectoryHandle(root, { limits: { maxEntries: 2 } });
    equal(pulls, 2, 'iterator consumption'); equal(closed, true, 'iterator released');
    equal(scan.candidates.length, 2, 'bounded candidates'); equal(scan.truncated, true, 'truthful stop status');
  });
  await test('FileList iterables stop before extra entries are pulled', async () => {
    let pulls = 0;
    function* files() { for (let i = 0; i < 100; i++) { pulls++; yield new File(['x'], `${i}.txt`); } }
    const scan = T.enumerateFileList(files(), { limits: { maxEntries: 2 } });
    equal(pulls, 2, 'iterator consumption'); equal(scan.candidates.length, 2, 'bounded candidates');
    equal(scan.truncated, true, 'truthful stop status');
  });
  await test('drop readers stop between batches and bound oversized batches', async () => {
    for (const batchSize of [1, 100]) {
      let reads = 0;
      const root = { isDirectory: true, isFile: false, name: 'fixture', createReader: () => ({ readEntries(done) {
        reads++; if (reads > 100) throw new Error('reader was not stopped');
        done(Array.from({ length: batchSize }, (_, i) => ({ isFile: true, isDirectory: false, name: `${reads}-${i}.txt`, file: done => done(new File(['x'], `${i}.txt`)) })));
      } }) };
      const scan = await T.enumerateDropEntries([root], { limits: { maxEntries: 3 } });
      equal(reads, batchSize === 1 ? 2 : 1, 'bounded reader calls');
      equal(scan.candidates.length, 2, 'root plus two entries'); equal(scan.truncated, true, 'truthful stop status');
    }
  });
  await test('cancel during full comparison stops subsequent reads', async () => {
    const controller = new AbortController();
    const block = new Blob([new Uint8Array(1024 * 1024)]);
    const a = new File(Array(65).fill(block), 'a.pdf'), b = new File([a], 'b.pdf');
    let reads = 0;
    const slice = b.slice.bind(b);
    b.slice = (start, end) => {
      const part = slice(start, end), read = part.arrayBuffer.bind(part);
      if (end - start <= 1024 * 1024) part.arrayBuffer = async () => { reads++; controller.abort(); return read(); };
      return part;
    };
    let aborted = false;
    try { await T.collectFolder('fixture', { candidates: [a, b].map(file => ({ path: file.name, name: file.name, open: async () => file })) }, { signal: controller.signal }); }
    catch (error) { aborted = error.name === 'AbortError'; }
    equal(aborted, true, 'comparison cancelled'); equal(reads, 1, 'no later comparison read');
  });
  return outcomes;
}
