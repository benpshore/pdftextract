/** Recursive folder imports for the browser app.
 *
 * Three folder sources feed one collector:
 *  - File System Access API (`showDirectoryPicker`, Chrome/Edge): `enumerateDirectoryHandle`.
 *  - `<input type="file" webkitdirectory>` (Safari/Firefox and everything else): `enumerateFileList`.
 *  - Drag-and-drop folders (`DataTransferItem.webkitGetAsEntry`): `enumerateDropItems`, which keeps
 *    calling `readEntries` until it returns an empty batch, as the directory-reader contract requires.
 *
 * `collectFolder` then applies the import policy: hidden/system files are skipped, every file is
 * classified by its extension (PDF first), unsupported types stay in the result with a reason instead
 * of vanishing, a per-file size limit and a queue-entry limit stop runaway imports with a plain message,
 * duplicates (confirmed equal bytes, with digests used only to find candidates) are dropped once, progress is
 * reported as "n of m" with bytes, and an AbortSignal cancels between entries. Files are never modified.
 */
export type FolderKind = 'pdf' | 'html' | 'text' | 'json' | 'xml' | 'css' | 'image' | 'office' | 'archive';
export type FolderFile = { file: File; path: string; size: number; kind: FolderKind; digest?: string };
export type FolderSkipKind = 'hidden' | 'unsupported' | 'too-large' | 'duplicate' | 'unreadable';
export type FolderSkip = { path: string; size: number | null; kind: FolderSkipKind; reason: string };
export type FolderProgress = { phase: 'scanning' | 'preparing'; label: string; found: number; prepared: number; total: number | null; bytes: number; path: string };
export type FolderLimits = { maxFileBytes: number; maxFiles: number; maxEntries: number; digestBytes: number };
export type FolderImport = { root: string; files: FolderFile[]; skipped: FolderSkip[]; scanned: number; bytes: number; truncated: boolean; message: string };
export type FolderOptions = { signal?: AbortSignal; onProgress?: (progress: FolderProgress) => void; limits?: Partial<FolderLimits> };
/** A file found while scanning. `open` is lazy so that limits can stop a scan before every file is read. */
export type Candidate = { path: string; name: string; size?: number; open: () => Promise<File> };

/** Structural subset of the File System Access API, so tests can supply plain objects. */
export type FileHandleLike = { kind: 'file'; name: string; getFile: () => Promise<File> };
export type DirectoryHandleLike = {
  kind: 'directory'; name: string;
  values: () => AsyncIterable<FileHandleLike | DirectoryHandleLike>;
  queryPermission?: (descriptor: { mode: 'read' | 'readwrite' }) => Promise<PermissionState>;
  requestPermission?: (descriptor: { mode: 'read' | 'readwrite' }) => Promise<PermissionState>;
};
/** Structural subset of the drag-and-drop `FileSystemEntry` family. */
export type DropEntryLike = {
  isFile: boolean; isDirectory: boolean; name: string;
  file?: (done: (file: File) => void, fail: (error: DOMException) => void) => void;
  createReader?: () => { readEntries: (done: (entries: DropEntryLike[]) => void, fail: (error: DOMException) => void) => void };
};
export type DropItemLike = { kind: string; webkitGetAsEntry?: () => DropEntryLike | null; getAsFile?: () => File | null };

export const DEFAULT_LIMITS: FolderLimits = {
  maxFileBytes: 512 * 1024 * 1024, // one file above this is listed as unsupported, never silently dropped
  maxFiles: 2000,                   // queue entries (supported + unsupported) per folder import
  maxEntries: 50000,                // directory entries examined before the scan stops
  digestBytes: 64 * 1024 * 1024,    // files up to this size are hashed completely for de-duplication
};
const EXTENSIONS: Record<string, FolderKind> = {
  pdf: 'pdf',
  html: 'html', htm: 'html', xhtml: 'html',
  txt: 'text', text: 'text', md: 'text', markdown: 'text',
  json: 'json', jsonl: 'json', ndjson: 'json',
  xml: 'xml', rss: 'xml', atom: 'xml',
  css: 'css',
  png: 'image', jpg: 'image', jpeg: 'image', webp: 'image', gif: 'image', bmp: 'image', tif: 'image', tiff: 'image',
  docx: 'office', pptx: 'office', xlsx: 'office', odt: 'office', ods: 'office', odp: 'office',
  zip: 'archive', tar: 'archive', tgz: 'archive', gz: 'archive',
};
const KIND_LABEL: Record<FolderKind, string> = { pdf: 'PDF', html: 'HTML', text: 'text', json: 'JSON', xml: 'XML', css: 'CSS', image: 'image', office: 'Office', archive: 'archive' };
export const SUPPORTED_SUMMARY = 'PDF, HTML, text, Markdown, JSON, XML, CSS, images, Office documents and archives';

/** macOS/Windows housekeeping files and dot-files are never imported from a folder. */
export function isHiddenName(name: string): boolean {
  return name.startsWith('.') || name.startsWith('~$') || name.startsWith('$') || /^(?:thumbs\.db|desktop\.ini|__MACOSX)$/i.test(name);
}
function hiddenSegment(path: string): string | null {
  return path.split('/').find(segment => isHiddenName(segment)) ?? null;
}
/** Extension-based classification; content sniffing still happens when the file is uploaded. */
export function classify(path: string): FolderKind | null {
  const name = path.split('/').pop() || '';
  if (/\.tar\.(?:gz|gzip)$/i.test(name)) return 'archive';
  const dot = name.lastIndexOf('.');
  if (dot <= 0 || dot === name.length - 1) return null;
  return EXTENSIONS[name.slice(dot + 1).toLowerCase()] ?? null;
}
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024, unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
function toHex(buffer: ArrayBuffer): string {
  return Array.from(new Uint8Array(buffer), byte => byte.toString(16).padStart(2, '0')).join('');
}
/** Candidate index only, never proof of equality. Hash at most 64 MiB in one allocation;
 * larger files use the first/last 4 MiB, then matching candidates get a bounded full comparison. */
export async function digestFile(file: Blob, digestBytes = DEFAULT_LIMITS.digestBytes, signal?: AbortSignal): Promise<string> {
  cancelled(signal);
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) throw new Error('This browser cannot compute content digests.');
  if (file.size <= Math.min(digestBytes, DEFAULT_LIMITS.digestBytes)) {
    const bytes = await file.arrayBuffer(); cancelled(signal);
    const hash = await subtle.digest('SHA-256', bytes); cancelled(signal);
    return `sha256:${toHex(hash)}`;
  }
  const edge = 4 * 1024 * 1024;
  const head = await file.slice(0, edge).arrayBuffer(); cancelled(signal);
  const tail = await file.slice(file.size - edge).arrayBuffer(); cancelled(signal);
  const joined = new Uint8Array(head.byteLength + tail.byteLength);
  joined.set(new Uint8Array(head), 0); joined.set(new Uint8Array(tail), head.byteLength);
  return `sampled-sha256:${file.size}:${toHex(await subtle.digest('SHA-256', joined))}`;
}
function cancelled(signal?: AbortSignal): void {
  if (signal?.aborted) throw signal.reason instanceof Error ? signal.reason : new DOMException('Folder import cancelled.', 'AbortError');
}
/** Digests (including complete hashes) are only an index; deletion requires byte identity.
 * Two 1 MiB buffers at a time, independent of file size. Original Blob/File bytes are untouched. */
async function equalFileBytes(a: Blob, b: Blob, signal?: AbortSignal): Promise<boolean> {
  if (a.size !== b.size) return false;
  const chunkBytes = 1024 * 1024;
  for (let offset = 0; offset < a.size; offset += chunkBytes) {
    cancelled(signal);
    const left = new Uint8Array(await a.slice(offset, offset + chunkBytes).arrayBuffer());
    cancelled(signal);
    const right = new Uint8Array(await b.slice(offset, offset + chunkBytes).arrayBuffer());
    cancelled(signal);
    const expected = Math.min(chunkBytes, a.size - offset);
    if (left.length !== expected || right.length !== expected) return false;
    for (let index = 0; index < left.length; index++) if (left[index] !== right[index]) return false;
  }
  return true;
}
function message(error: unknown): string { return error instanceof Error ? error.message : String(error); }
function pathSort(a: { path: string }, b: { path: string }): number { return a.path < b.path ? -1 : a.path > b.path ? 1 : 0; }

/** Chrome/Edge: walk a directory handle from `showDirectoryPicker`. Hidden directories are pruned. */
export type Scan = { candidates: Candidate[]; hidden?: FolderSkip[]; unreadable?: FolderSkip[]; truncated?: boolean; examined?: number };
export async function enumerateDirectoryHandle(root: DirectoryHandleLike, options: FolderOptions = {}): Promise<Scan> {
  const limits = { ...DEFAULT_LIMITS, ...options.limits };
  const candidates: Candidate[] = [], hidden: FolderSkip[] = [];
  const stack: { handle: DirectoryHandleLike; path: string }[] = [{ handle: root, path: root.name }];
  let examined = 0, truncated = false;
  while (stack.length && !truncated) {
    cancelled(options.signal);
    if (examined >= limits.maxEntries) { truncated = true; break; }
    const current = stack.pop()!;
    const children: (FileHandleLike | DirectoryHandleLike)[] = [];
    for await (const child of current.handle.values()) {
      cancelled(options.signal);
      children.push(child); examined++;
      options.onProgress?.({ phase: 'scanning', label: `Scanning ${current.path}`, found: candidates.length, prepared: 0, total: null, bytes: 0, path: current.path });
      // Stop consuming the iterator at the budget, before collecting/sorting more entries.
      if (examined >= limits.maxEntries) { truncated = true; break; }
    }
    children.sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
    const directories: { handle: DirectoryHandleLike; path: string }[] = [];
    for (const child of children) {
      const path = `${current.path}/${child.name}`;
      if (isHiddenName(child.name)) { hidden.push({ path, size: null, kind: 'hidden', reason: 'Hidden or system file.' }); continue; }
      if (child.kind === 'directory') directories.push({ handle: child, path });
      else candidates.push({ path, name: child.name, open: () => child.getFile() });
      options.onProgress?.({ phase: 'scanning', label: `Scanning ${current.path}`, found: candidates.length, prepared: 0, total: null, bytes: 0, path });
    }
    for (let index = directories.length - 1; index >= 0; index--) stack.push(directories[index]);
  }
  return { candidates, hidden, truncated, examined: Math.min(examined, limits.maxEntries) };
}
/** Safari/Firefox and everyone else: a `webkitdirectory` input's FileList (relative paths are set by the browser). */
export function enumerateFileList(files: Iterable<File>, options: FolderOptions = {}): Scan {
  const limits = { ...DEFAULT_LIMITS, ...options.limits };
  const candidates: Candidate[] = [], hidden: FolderSkip[] = [];
  let examined = 0, truncated = false;
  cancelled(options.signal);
  if (limits.maxEntries <= 0) return { candidates, hidden, truncated: true, examined };
  for (const file of files) {
    cancelled(options.signal); examined++;
    const path = (file.webkitRelativePath || file.name).replace(/\\/g, '/').replace(/^\/+/, '');
    const segment = hiddenSegment(path);
    if (segment) hidden.push({ path, size: file.size, kind: 'hidden', reason: `Hidden or system file (${segment}).` });
    else candidates.push({ path, name: file.name, size: file.size, open: async () => file });
    if (examined >= limits.maxEntries) { truncated = true; break; }
  }
  candidates.sort(pathSort);
  return { candidates, hidden, truncated, examined };
}
/** Drag-and-drop: call synchronously inside the drop handler, because `webkitGetAsEntry` only works before the event ends. */
export function dropEntries(items: Iterable<DropItemLike>): { entries: DropEntryLike[]; files: File[]; hasDirectory: boolean } {
  const entries: DropEntryLike[] = [], files: File[] = [];
  for (const item of items) {
    if (item.kind !== 'file') continue;
    const entry = item.webkitGetAsEntry?.();
    if (entry) entries.push(entry);
    else { const file = item.getAsFile?.(); if (file) files.push(file); }
  }
  return { entries, files, hasDirectory: entries.some(entry => entry.isDirectory) };
}
async function readEntriesWithinBudget(entry: DropEntryLike, budget: number, signal?: AbortSignal): Promise<{ children: DropEntryLike[]; truncated: boolean }> {
  const reader = entry.createReader?.();
  const children: DropEntryLike[] = [];
  if (!reader) return { children, truncated: false };
  while (children.length < budget) {
    cancelled(signal);
    const batch = await new Promise<DropEntryLike[]>((resolve, reject) => {
      const abort = () => { cleanup(); reject(signal?.reason ?? new DOMException('Folder import cancelled.', 'AbortError')); };
      const cleanup = () => signal?.removeEventListener('abort', abort);
      signal?.addEventListener('abort', abort, { once: true });
      if (signal?.aborted) { abort(); return; }
      try { reader.readEntries(entries => { cleanup(); resolve(entries); }, error => { cleanup(); reject(error); }); }
      catch (error) { cleanup(); reject(error); }
    });
    cancelled(signal);
    if (!batch.length) return { children, truncated: false };
    // The browser owns each returned batch; retain only the budgeted prefix and never request more.
    const count = Math.min(batch.length, budget - children.length);
    for (let index = 0; index < count; index++) children.push(batch[index]);
  }
  return { children, truncated: true };
}
export async function enumerateDropEntries(roots: DropEntryLike[], options: FolderOptions = {}): Promise<Scan> {
  const limits = { ...DEFAULT_LIMITS, ...options.limits };
  const candidates: Candidate[] = [], hidden: FolderSkip[] = [], unreadable: FolderSkip[] = [];
  const stack: { entry: DropEntryLike; path: string }[] = [];
  for (let index = Math.min(roots.length, limits.maxEntries) - 1; index >= 0; index--) stack.push({ entry: roots[index], path: roots[index].name });
  let examined = 0, truncated = roots.length > limits.maxEntries;
  while (stack.length) {
    const current = stack.pop()!;
    cancelled(options.signal);
    if (++examined > limits.maxEntries) { truncated = true; break; }
    if (isHiddenName(current.entry.name)) { hidden.push({ path: current.path, size: null, kind: 'hidden', reason: 'Hidden or system file.' }); continue; }
    if (current.entry.isDirectory) {
      let children: DropEntryLike[];
      try {
        const read = await readEntriesWithinBudget(current.entry, limits.maxEntries - examined - stack.length, options.signal);
        children = read.children; truncated ||= read.truncated;
      }
      catch (error) { cancelled(options.signal); unreadable.push({ path: current.path, size: null, kind: 'unreadable', reason: `Folder could not be read: ${message(error)}` }); continue; }
      children.sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
      for (let index = children.length - 1; index >= 0; index--) stack.push({ entry: children[index], path: `${current.path}/${children[index].name}` });
      options.onProgress?.({ phase: 'scanning', label: `Scanning ${current.path}`, found: candidates.length, prepared: 0, total: null, bytes: 0, path: current.path });
    } else if (current.entry.isFile && current.entry.file) {
      const entry = current.entry;
      candidates.push({ path: current.path, name: entry.name, open: () => new Promise<File>((resolve, reject) => entry.file!(resolve, reject)) });
    }
  }
  return { candidates, hidden, unreadable, truncated, examined: Math.min(examined, limits.maxEntries) };
}

/** Apply the import policy to scanned candidates. PDFs are queued first; everything else keeps scan order. */
export async function collectFolder(root: string, scan: Scan, options: FolderOptions = {}): Promise<FolderImport> {
  const limits = { ...DEFAULT_LIMITS, ...options.limits };
  const files: FolderFile[] = [], skipped: FolderSkip[] = [...(scan.hidden ?? []), ...(scan.unreadable ?? [])];
  const bySize = new Map<number, FolderFile[]>();
  // `reported` starts at -Infinity so the first file is always announced, whatever the process uptime; later events are throttled to one per 100 ms.
  let bytes = 0, prepared = 0, reported = -Infinity, truncated = !!scan.truncated, stoppedAt = 0;
  const total = scan.candidates.length;
  const report = (path: string, force = false) => {
    const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
    if (!force && now - reported < 100) return;
    reported = now;
    options.onProgress?.({ phase: 'preparing', label: `Preparing ${prepared} of ${total} files (${formatBytes(bytes)})`, found: total, prepared, total, bytes, path });
  };
  const queued = () => files.length + skipped.filter(entry => entry.kind === 'unsupported' || entry.kind === 'too-large').length;
  for (const candidate of scan.candidates) {
    cancelled(options.signal);
    if (queued() >= limits.maxFiles) { truncated = true; stoppedAt = prepared; break; }
    prepared++;
    const path = candidate.path;
    const kind = classify(path);
    if (!kind) { skipped.push({ path, size: candidate.size ?? null, kind: 'unsupported', reason: `Unsupported file type. Folder import handles ${SUPPORTED_SUMMARY}; add this file on its own to store the original.` }); report(path); continue; }
    let file: File;
    try { file = await candidate.open(); }
    catch (error) { cancelled(options.signal); skipped.push({ path, size: candidate.size ?? null, kind: 'unreadable', reason: `Could not read this file: ${message(error)}` }); report(path); continue; }
    if (file.size > limits.maxFileBytes) { skipped.push({ path, size: file.size, kind: 'too-large', reason: `File is ${formatBytes(file.size)}; folder import accepts files up to ${formatBytes(limits.maxFileBytes)}. Add it on its own to import it anyway.` }); report(path); continue; }
    const entry: FolderFile = { file, path, size: file.size, kind };
    const sameSize = bySize.get(file.size);
    if (sameSize) {
      // Only files of equal size can be identical; hash lazily so most scans never read file bodies here.
      try {
        entry.digest = await digestFile(file, limits.digestBytes, options.signal);
        let twin: FolderFile | undefined;
        for (const other of sameSize) {
          other.digest ??= await digestFile(other.file, limits.digestBytes, options.signal);
          if (other.digest === entry.digest && await equalFileBytes(other.file, file, options.signal)) { twin = other; break; }
        }
        if (twin) { skipped.push({ path, size: file.size, kind: 'duplicate', reason: `Same content as ${twin.path}.` }); report(path); continue; }
      } catch (error) { cancelled(options.signal); /* a digest failure keeps both copies rather than guessing */ void error; }
      sameSize.push(entry);
    } else bySize.set(file.size, [entry]);
    files.push(entry); bytes += file.size;
    report(path);
  }
  report('', true);
  cancelled(options.signal); // a Cancel pressed during the last progress update still discards the batch
  const pdfs = files.filter(entry => entry.kind === 'pdf'), others = files.filter(entry => entry.kind !== 'pdf');
  const ordered = [...pdfs, ...others];
  const counts = (kind: FolderSkipKind) => skipped.filter(entry => entry.kind === kind).length;
  const parts = [`${ordered.length} file${ordered.length === 1 ? '' : 's'} queued from ${root} (${formatBytes(bytes)})`];
  if (pdfs.length) parts.push(`${pdfs.length} PDF${pdfs.length === 1 ? '' : 's'} first`);
  const detail: string[] = [];
  if (counts('unsupported')) detail.push(`${counts('unsupported')} unsupported`);
  if (counts('too-large')) detail.push(`${counts('too-large')} too large`);
  if (counts('duplicate')) detail.push(`${counts('duplicate')} duplicate${counts('duplicate') === 1 ? '' : 's'} skipped`);
  if (counts('hidden')) detail.push(`${counts('hidden')} hidden skipped`);
  if (counts('unreadable')) detail.push(`${counts('unreadable')} unreadable`);
  let text = parts.join('; ') + (detail.length ? `; ${detail.join(', ')}` : '') + '.';
  if (truncated) text += stoppedAt ? ` Stopped at the ${limits.maxFiles}-file limit after ${stoppedAt} of ${total} files; import the remaining subfolders separately.` : ` The scan stopped after ${scan.examined ?? limits.maxEntries} entries; import the remaining subfolders separately.`;
  return { root, files: ordered, skipped, scanned: total, bytes, truncated, message: text };
}

/** Chrome/Edge only. Returns null when the user closes the picker. The `id` lets the browser reopen the last location. */
export function supportsDirectoryPicker(): boolean {
  return typeof window !== 'undefined' && typeof (window as Window & { showDirectoryPicker?: unknown }).showDirectoryPicker === 'function';
}
export async function pickDirectory(startIn?: DirectoryHandleLike): Promise<DirectoryHandleLike | null> {
  const picker = (window as Window & { showDirectoryPicker?: (options: { mode: 'read'; id: string; startIn?: DirectoryHandleLike }) => Promise<DirectoryHandleLike> }).showDirectoryPicker;
  if (!picker) return null;
  try { return await picker.call(window, { mode: 'read', id: 'tpe-folder-import', ...(startIn ? { startIn } : {}) }); }
  catch (error) { if (error instanceof DOMException && error.name === 'AbortError') return null; throw error; }
}
/** Re-use a handle picked earlier in this session; the browser only re-prompts when its grant has lapsed. */
export async function ensureReadable(handle: DirectoryHandleLike): Promise<boolean> {
  if (!handle.queryPermission) return true;
  if (await handle.queryPermission({ mode: 'read' }) === 'granted') return true;
  if (!handle.requestPermission) return false;
  return await handle.requestPermission({ mode: 'read' }) === 'granted';
}
export function describeKind(kind: FolderKind): string { return KIND_LABEL[kind]; }
