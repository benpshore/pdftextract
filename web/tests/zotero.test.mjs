// Zotero module tests: pure helpers, the browser API client against a mocked
// fetch (no network), the send run including the three-step file upload,
// and the panel rendered in JSDOM through the full connect → send flow.
// Run from web/: `node tests/zotero.test.mjs`.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { JSDOM, VirtualConsole } from 'jsdom';

const require = createRequire(import.meta.url);
const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');

// Bundle before any browser global exists: esbuild's own API must see plain Node.
const built = await build({
  stdin: { contents: "export { createRoot } from 'react-dom/client'; export { act, createElement } from 'react'; export * from './lib/zotero/index';", resolveDir: web, loader: 'ts', sourcefile: 'zotero-test-entry.ts' },
  bundle: true, write: false, format: 'esm', platform: 'browser', jsx: 'automatic', logLevel: 'silent',
  define: { 'process.env.NODE_ENV': '"development"' },
});
const dom = new JSDOM('<!doctype html><html><body><main id="root"></main></body></html>', { url: 'https://tpe.test/', pretendToBeVisual: true, virtualConsole: new VirtualConsole() });
const { window } = dom;
globalThis.window = window;
for (const key of ['document', 'HTMLElement', 'Element', 'Node', 'Event', 'HTMLInputElement', 'HTMLSelectElement', 'HTMLTextAreaElement', 'HTMLButtonElement', 'HTMLFormElement', 'Text', 'DocumentFragment', 'sessionStorage', 'requestAnimationFrame', 'cancelAnimationFrame']) globalThis[key] = window[key];
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const Z = await import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].text).toString('base64')}`);

const KEY = 'P9NiFoyLeZu2bZNvvuQPDWsd';
const API = 'https://api.zotero.org';
const NEW_KEYS = { journalArticle: 'ABCD2345', preprint: 'PREP2345', note: 'NOTE2345', linked_url: 'LINK2345', imported_file: 'FILE2345' };

/** A recorded-fixture Zotero API behind a fetch function; `route` may intercept first. */
function server(route = () => null) {
  const calls = [];
  const fetchImpl = async (url, init = {}) => {
    const headers = new Headers(init.headers || {});
    const call = { url, method: init.method || 'GET', headers: Object.fromEntries(headers.entries()), body: init.body };
    calls.push(call);
    const intercepted = route(call, calls);
    if (intercepted) return intercepted;
    if (url.startsWith('https://storage.example/')) return new Response('', { status: 201 });
    assert.equal(headers.get('zotero-api-key'), KEY, 'every API request carries the key header');
    assert.equal(headers.get('zotero-api-version'), '3');
    const path = url.slice(API.length);
    if (path === '/keys/current') return Response.json({ key: KEY, userID: 475425, username: 'ada', displayName: 'Ada L.', access: { user: { library: true, files: true, notes: true, write: true }, groups: { all: { library: true, write: false }, 123: { library: true, write: true } } } });
    if (path === '/users/475425/groups?limit=100') return Response.json([{ id: 123, version: 4, meta: { numItems: 1 }, data: { id: 123, version: 4, name: 'Reading Group', libraryEditing: 'members' } }]);
    if (path === '/users/475425/collections?limit=100') return Response.json([{ key: 'COLL2345', version: 3, data: { key: 'COLL2345', name: 'Reading', parentCollection: false } }], { headers: { Link: `<${API}/users/475425/collections?limit=100&start=1>; rel="next", <${API}/users/475425/collections?limit=100&start=1>; rel="last"` } });
    if (path === '/users/475425/collections?limit=100&start=1') return Response.json([{ key: 'SUBC2345', version: 4, data: { key: 'SUBC2345', name: 'Sub', parentCollection: 'COLL2345' } }]);
    if (path === '/users/475425/items' && call.method === 'POST') {
      assert.match(headers.get('zotero-write-token') || '', /^[0-9a-f]{32}$/);
      assert.equal(headers.get('content-type'), 'application/json');
      const items = JSON.parse(init.body);
      const successful = {}, success = {};
      items.forEach((item, index) => { const key = NEW_KEYS[item.linkMode || item.itemType]; successful[index] = { key, version: 10, data: item }; success[index] = key; });
      return Response.json({ successful, success, unchanged: {}, failed: {} }, { headers: { 'Last-Modified-Version': '10' } });
    }
    if (path === '/users/475425/items/FILE2345/file' && call.method === 'POST') {
      assert.equal(headers.get('content-type'), 'application/x-www-form-urlencoded');
      if (String(init.body).startsWith('upload=')) { assert.equal(headers.get('if-none-match'), '*'); return new Response(null, { status: 204 }); }
      assert.equal(headers.get('if-none-match'), '*');
      return Response.json({ url: 'https://storage.example/', contentType: 'multipart/form-data; boundary=X9', prefix: '--X9\r\nContent-Disposition: form-data; name="file"\r\n\r\n', suffix: '\r\n--X9--', uploadKey: 'UPLOAD1' });
    }
    return new Response('Not found', { status: 404 });
  };
  return { calls, fetchImpl };
}

const article = {
  title: 'Attention Is All You Need in Zotero', text: 'See https://doi.org/10.1000/ref.one and doi:10.1000/REF.two', engine: 'PDF Oxide 0.3', status: 'ready', warnings: [],
  metadata: { title: 'Attention Is All You Need in Zotero', authors: ['Lovelace, Ada', 'Zotero Consortium'], date: '2019-03', journal: 'Journal of Tests', doi: 'https://doi.org/10.1000/ABC.123' },
  links: [
    { url: 'https://doi.org/10.1000/ABC.123', doi: '10.1000/abc.123', page: 1, kind: 'doi' },
    { url: 'https://doi.org/10.1000/ref.one', doi: '10.1000/ref.one', page: 7, kind: 'doi', label: 'Ref 1 <b>' },
    { url: 'doi:10.1000/REF.two', page: 8, kind: 'doi' },
    { url: 'https://doi.org/10.1000/ref.one', page: 9, kind: 'doi' },
  ],
};

const cases = [];
const test = (name, run) => cases.push([name, run]);

test('MD5 matches RFC 1321 vectors', () => {
  assert.equal(Z.md5Hex(new Uint8Array()), 'd41d8cd98f00b204e9800998ecf8427e');
  assert.equal(Z.md5Hex(new TextEncoder().encode('abc')), '900150983cd24fb0d6963f7d28e17f72');
  assert.equal(Z.md5Hex(new TextEncoder().encode('message digest')), 'f96b697d7cb7938d525a2f31aaf161d0');
  assert.equal(Z.md5Hex(new TextEncoder().encode('12345678901234567890123456789012345678901234567890123456789012345678901234567890')), '57edf4a22be3c955ac49da2e2107b67a');
});

test('Link headers and write tokens', () => {
  const link = `<${API}/users/1/items?limit=30&start=30>; rel="next",\n <${API}/users/1/items?itemKey=A,B&start=5040>; rel="last"`;
  assert.equal(Z.nextLink(link), `${API}/users/1/items?limit=30&start=30`);
  assert.equal(Z.parseLinkHeader(link)[1].url, `${API}/users/1/items?itemKey=A,B&start=5040`);
  assert.equal(Z.nextLink('<https://x/>; rel=prev'), null);
  assert.equal(Z.nextLink(null), null);
  assert.match(Z.newWriteToken(), /^[0-9a-f]{32}$/);
  assert.notEqual(Z.newWriteToken(), Z.newWriteToken());
});

test('Identifiers are normalised, never invented', () => {
  assert.equal(Z.normalizeDoi('https://doi.org/10.1000/ABC.123.'), '10.1000/abc.123');
  assert.equal(Z.normalizeDoi('doi:10.1000/x'), '10.1000/x');
  assert.equal(Z.normalizeDoi('not a doi'), null);
  assert.equal(Z.normalizeArxivId('arXiv:2502.00857v2'), '2502.00857');
  assert.equal(Z.normalizeArxivId('https://arxiv.org/abs/hep-th/9901001'), 'hep-th/9901001');
  assert.equal(Z.normalizeArxivId('12'), null);
  assert.deepEqual(Z.creatorFromDisplay('Lovelace, Ada'), { creatorType: 'author', firstName: 'Ada', lastName: 'Lovelace' });
  assert.deepEqual(Z.creatorFromDisplay('Zotero Consortium'), { creatorType: 'author', name: 'Zotero Consortium' });
});

test('Article metadata, references and the item body', () => {
  const meta = Z.articleMetadata(article, 'https://example.org/a.pdf');
  assert.equal(meta.doi, '10.1000/abc.123');
  assert.equal(meta.venue, 'Journal of Tests');
  assert.equal(meta.url, 'https://example.org/a.pdf');
  assert.deepEqual(meta.authors, ['Lovelace, Ada', 'Zotero Consortium']);
  assert.equal(Z.suggestItemType(meta), 'journalArticle');
  assert.equal(Z.suggestItemType({ ...meta, venue: '', doi: '', arxivId: '2502.00857' }), 'preprint');
  assert.equal(Z.suggestItemType({ ...meta, venue: '', doi: '' }), 'webpage');
  const refs = Z.references(article, meta.doi);
  assert.deepEqual(refs.map(ref => ref.doi), ['10.1000/ref.one', '10.1000/ref.two']);
  const item = Z.buildItem(meta, { itemType: 'journalArticle', collections: ['SUBC2345'], tags: ['tpe', ' ', 'read'] });
  assert.equal(item.publicationTitle, 'Journal of Tests');
  assert.equal(item.DOI, '10.1000/abc.123');
  assert.deepEqual(item.tags, [{ tag: 'tpe' }, { tag: 'read' }]);
  assert.deepEqual(item.collections, ['SUBC2345']);
  assert.deepEqual(item.creators[0], { creatorType: 'author', firstName: 'Ada', lastName: 'Lovelace' });
  const book = Z.buildItem(meta, { itemType: 'book', collections: [], tags: [] });
  assert.equal(book.DOI, undefined);
  assert.equal(book.extra, 'DOI: 10.1000/abc.123');
  assert.equal(book.publisher, 'Journal of Tests');
  const template = { itemType: 'webpage', title: '', creators: [], websiteTitle: '', url: '', date: '', extra: '', tags: [], collections: [], relations: {} };
  const page = Z.buildItem(meta, { itemType: 'webpage', collections: [], tags: [], template });
  assert.equal(page.abstractNote, undefined, 'fields the template lacks are not set');
  assert.equal(page.websiteTitle, 'Journal of Tests');
  assert.equal(page.extra, 'DOI: 10.1000/abc.123');
  const note = Z.referencesNoteHtml(refs, 'PDF <Oxide>');
  assert.match(note, /Extracted by PDF &lt;Oxide&gt;\./);
  assert.match(note, /<li>Ref 1 &lt;b&gt; — <a href="https:\/\/doi.org\/10.1000\/ref.one">10.1000\/ref.one<\/a> <span>\(page 7\)<\/span><\/li>/);
  assert.ok(!note.includes('<b>'));
  assert.deepEqual(Z.orderCollections([{ key: 'B', name: 'Sub', parent: 'A' }, { key: 'A', name: 'Top', parent: null }, { key: 'C', name: 'Orphan', parent: 'ZZ' }]).map(entry => entry.name + entry.depth), ['Top0', 'Sub1', 'Orphan0']);
});

test('Status mapping, retry delays and write results', () => {
  assert.equal(Z.statusError(403, '').kind, 'forbidden');
  assert.equal(Z.statusError(412, '').kind, 'precondition');
  assert.equal(Z.statusError(409, '').kind, 'conflict');
  assert.equal(Z.statusError(413, 'big').kind, 'too-large');
  const limited = Z.statusError(429, '', new Headers({ 'Retry-After': '7' }));
  assert.equal(limited.retryAfterSeconds, 7);
  assert.equal(Z.retryDelay(limited, 1, 3, 60000), 7000);
  assert.equal(Z.retryDelay(limited, 3, 3, 60000), null);
  assert.equal(Z.retryDelay(Z.statusError(503, ''), 2, 3, 60000), 2000);
  assert.equal(Z.retryDelay(Z.statusError(400, ''), 1, 3, 60000), null);
  const result = Z.parseWriteResult({ success: { 0: 'AAAA2222' }, unchanged: { 1: 'BBBB3333' }, failed: { 2: { code: 400, message: 'Invalid field' } } }, 10);
  assert.equal(Z.keyAt(result, 0), 'AAAA2222');
  assert.equal(Z.keyAt(result, 1), 'BBBB3333');
  assert.throws(() => Z.keyAt(result, 2), error => error.kind === 'write-failed' && /Invalid field/.test(error.message));
  assert.deepEqual(Z.parseUploadAuthorization({ exists: 1 }), { exists: true });
  const auth = Z.parseUploadAuthorization({ url: 'https://s/', params: { key: 'k', policy: 'p' }, uploadKey: 'u' });
  assert.deepEqual(auth.target.params, { key: 'k', policy: 'p' });
  const info = Z.parseKeyInfo({ userID: 7, access: { user: { library: true }, groups: { all: { library: true, write: true } } } });
  assert.equal(Z.accessFor(info, { type: 'group', id: 5 }).write, true);
  assert.equal(Z.accessFor(info, { type: 'user', id: 7 }).write, false);
  assert.equal(Z.itemWebUrl({ userId: 7, username: 'ada', user: info.user, groups: {} }, { type: 'user', id: 7 }, 'ABCD2345'), 'https://www.zotero.org/ada/items/ABCD2345');
  assert.equal(Z.itemWebUrl(info, { type: 'group', id: 5 }, 'ABCD2345'), 'https://www.zotero.org/groups/5/items/ABCD2345');
  assert.equal(Z.itemWebUrl(info, { type: 'user', id: 7 }, '../etc'), null);
});

test('API client: headers, paging, write token, Backoff and Retry-After', async () => {
  const { calls, fetchImpl } = server();
  const slept = [];
  const api = new Z.ZoteroApi(KEY, { fetch: fetchImpl, sleep: async ms => { slept.push(ms); } });
  const info = await api.keyInfo();
  assert.equal(info.userId, 475425);
  assert.equal(calls[0].url, `${API}/keys/current`, 'the key is never put in a URL');
  const collections = await api.collections({ type: 'user', id: 475425 });
  assert.deepEqual(collections.map(entry => entry.key), ['COLL2345', 'SUBC2345']);
  assert.equal(calls.length, 3, 'the next page was followed once');
  const groups = await api.groups(475425);
  assert.equal(groups[0].name, 'Reading Group');
  const written = await api.createItems({ type: 'user', id: 475425 }, [{ itemType: 'journalArticle', title: 'T' }]);
  assert.equal(Z.keyAt(written, 0), 'ABCD2345');
  assert.equal(written.lastModifiedVersion, 10);
  assert.ok(calls.every(call => !call.url.includes(KEY) && !(typeof call.body === 'string' && call.body.includes(KEY))), 'the key appears in no URL or body');
  assert.deepEqual(slept, []);

  let attempts = 0;
  const limited = server(call => {
    if (!call.url.endsWith('/keys/current')) return null;
    attempts++;
    if (attempts === 1) return new Response('', { status: 429, headers: { 'Retry-After': '2' } });
    if (attempts === 2) return new Response('', { status: 503 });
    return null;
  });
  const retrying = new Z.ZoteroApi(KEY, { fetch: limited.fetchImpl, sleep: async ms => { slept.push(ms); }, maxAttempts: 4 });
  assert.equal((await retrying.keyInfo()).username, 'ada');
  assert.deepEqual(slept, [2000, 2000], 'Retry-After then exponential backoff');
  slept.length = 0;
  const backing = server(call => call.url.endsWith('/keys/current') ? Response.json({ userID: 1, access: {} }, { headers: { Backoff: '3' } }) : null);
  const polite = new Z.ZoteroApi(KEY, { fetch: backing.fetchImpl, sleep: async ms => { slept.push(ms); } });
  await polite.keyInfo();
  assert.deepEqual(slept, [], 'no wait before the first request');
  await polite.keyInfo();
  assert.equal(slept.length, 1);
  assert.ok(slept[0] > 2500 && slept[0] <= 3000, 'the Backoff pause is honoured before the next request');

  const forbidden = new Z.ZoteroApi('wrong', { fetch: async () => new Response('', { status: 403 }), sleep: async () => {} });
  await assert.rejects(forbidden.keyInfo(), error => error.kind === 'forbidden');
  const stale = new Z.ZoteroApi(KEY, { fetch: async () => new Response('', { status: 412 }), sleep: async () => {} });
  await assert.rejects(stale.updateItem({ type: 'user', id: 1 }, 'ABCD2345', 3, { title: 'x' }), error => error.kind === 'precondition' && error.status === 412);
  let failures = 0;
  const offline = new Z.ZoteroApi(KEY, { fetch: async () => { failures++; throw new TypeError('Failed to fetch'); }, sleep: async () => {} });
  await assert.rejects(offline.keyInfo(), error => error.kind === 'network' && /Failed to fetch/.test(error.message));
  assert.equal(failures, 3, 'network failures are retried up to the attempt limit');
  await assert.rejects(api.collections({ type: 'user', id: 475425 }).then(() => new Z.ZoteroApi(KEY, { fetch: async () => Response.json([], { headers: { Link: '<https://evil.example/users/1/collections>; rel="next"' } }), sleep: async () => {} }).collections({ type: 'user', id: 1 })), /outside the API/);
});

test('Send run: item, children, and the three-step file upload', async () => {
  const { calls, fetchImpl } = server();
  const api = new Z.ZoteroApi(KEY, { fetch: fetchImpl, sleep: async () => {} });
  const info = await api.keyInfo();
  const progress = [];
  const meta = Z.articleMetadata(article, 'https://example.org/a.pdf');
  const outcome = await Z.sendToZotero(api, info, {
    library: { type: 'user', id: 475425 },
    item: Z.buildItem(meta, { itemType: 'journalArticle', collections: [], tags: ['tpe'] }),
    noteHtml: Z.referencesNoteHtml(Z.references(article, meta.doi)),
    link: { url: 'https://example.org/a.pdf', title: 'Source' },
    file: { blob: new Blob(['%PDF-1.7 test'], { type: 'application/pdf' }), filename: 'a.pdf', contentType: 'application/pdf', title: 'Full Text PDF' },
  }, message => progress.push(message));
  assert.deepEqual(outcome, { itemKey: 'ABCD2345', noteKey: 'NOTE2345', linkKey: 'LINK2345', fileKey: 'FILE2345', webUrl: 'https://www.zotero.org/ada/items/ABCD2345', warnings: [] });
  assert.equal(progress.at(-1), 'Saved to Zotero.');
  const writes = calls.filter(call => call.url === `${API}/users/475425/items`);
  assert.equal(writes.length, 3);
  assert.deepEqual(JSON.parse(writes[1].body).map(child => child.itemType + ':' + (child.linkMode || '') + ':' + child.parentItem), ['note::ABCD2345', 'attachment:linked_url:ABCD2345']);
  const attachment = JSON.parse(writes[2].body)[0];
  assert.equal(attachment.linkMode, 'imported_file');
  assert.equal(attachment.md5, Z.md5Hex(new TextEncoder().encode('%PDF-1.7 test')));
  assert.equal(attachment.filename, 'a.pdf');
  const authorize = calls.find(call => call.url.endsWith('/items/FILE2345/file') && String(call.body).startsWith('md5='));
  assert.match(authorize.body, /^md5=[0-9a-f]{32}&filename=a\.pdf&filesize=13&mtime=\d+$/);
  const storage = calls.find(call => call.url.startsWith('https://storage.example/'));
  assert.equal(storage.method, 'POST');
  assert.equal(storage.headers['zotero-api-key'], undefined, 'no Zotero header reaches the storage host');
  assert.equal(storage.headers['content-type'], 'multipart/form-data; boundary=X9');
  assert.equal(await storage.body.text(), '--X9\r\nContent-Disposition: form-data; name="file"\r\n\r\n%PDF-1.7 test\r\n--X9--');
  const register = calls.find(call => call.url.endsWith('/items/FILE2345/file') && String(call.body).startsWith('upload='));
  assert.equal(register.body, 'upload=UPLOAD1');
  assert.ok(calls.indexOf(storage) < calls.indexOf(register), 'registration follows the upload');

  // A failed upload keeps the item and reports a warning.
  const blocked = server(call => call.url.startsWith('https://storage.example/') ? (() => { throw new TypeError('Failed to fetch'); })() : null);
  const partial = await Z.sendToZotero(new Z.ZoteroApi(KEY, { fetch: blocked.fetchImpl, sleep: async () => {} }), info, {
    library: { type: 'user', id: 475425 }, item: { itemType: 'journalArticle', title: 'T' },
    file: { blob: new Blob(['x']), filename: 'x.pdf', contentType: 'application/pdf', title: 'x' },
  });
  assert.equal(partial.itemKey, 'ABCD2345');
  assert.equal(partial.fileKey, 'FILE2345');
  assert.match(partial.warnings[0], /file was not uploaded.*blocked the cross-origin upload/);
});

test('Key storage: session only here; IndexedDB absence is an explicit error', async () => {
  await Z.storeKey(KEY, 'session');
  assert.deepEqual((await Z.loadStoredKey()).persistence, 'session');
  assert.equal(JSON.parse(window.sessionStorage.getItem('tpe-zotero-api-key')).key, KEY);
  await Z.clearStoredKey();
  assert.equal(await Z.loadStoredKey(), null);
  await assert.rejects(Z.storeKey(KEY, 'persistent'), /IndexedDB is not available/);
});

// Panel flow in JSDOM.
const { act, createRoot, createElement } = Z;
const root = document.getElementById('root');
const setValue = (element, value) => {
  const proto = element instanceof window.HTMLSelectElement ? window.HTMLSelectElement.prototype : element instanceof window.HTMLTextAreaElement ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(element, value);
  element.dispatchEvent(new window.Event(element instanceof window.HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
};
async function until(check, label) {
  for (let i = 0; i < 400; i++) {
    if (check()) return;
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
  }
  throw new Error('Timed out waiting for ' + label);
}
const byLabel = text => {
  const label = [...document.querySelectorAll('label')].find(element => element.textContent.trim().startsWith(text));
  assert.ok(label, 'label ' + text);
  return label.control || label.querySelector('input,select,textarea');
};
const button = text => [...document.querySelectorAll('button')].find(element => element.textContent.trim() === text);

test('Panel: connect with a session-only key, pick a collection, send, forget', async () => {
  const { calls, fetchImpl } = server();
  const reactRoot = createRoot(root);
  await act(async () => { reactRoot.render(createElement(Z.ZoteroPanel, { article, sourceUrl: 'https://example.org/a.pdf', original: { blob: new Blob(['%PDF']), name: 'a.pdf', contentType: 'application/pdf' }, apiOptions: { fetch: fetchImpl, sleep: async () => {} } })); });
  await until(() => byLabel('Zotero API key'), 'the key form');
  const keyInput = byLabel('Zotero API key');
  assert.equal(keyInput.type, 'password');
  assert.equal(keyInput.getAttribute('autocomplete'), 'off');
  assert.equal(button('Connect').disabled, true, 'Connect waits for a key');
  assert.equal(byLabel('Until this tab is closed').checked, true, 'session-only is the default');
  await act(async () => { setValue(keyInput, ' ' + KEY + ' '); });
  assert.equal(button('Connect').disabled, false);
  await act(async () => { button('Connect').click(); });
  await until(() => /Connected as Ada L\./.test(root.textContent), 'the connection');
  assert.equal(JSON.parse(window.sessionStorage.getItem('tpe-zotero-api-key')).key, KEY, 'the key is kept in sessionStorage');
  assert.ok(calls.every(call => call.url.startsWith(API + '/')), 'only api.zotero.org was called');
  const library = byLabel('Library');
  assert.deepEqual([...library.options].map(option => option.textContent), ['My library (ada)', 'Reading Group']);
  await until(() => byLabel('Collection').options.length === 3, 'collections');
  assert.deepEqual([...byLabel('Collection').options].map(option => option.textContent), ['No collection (library root)', 'Reading', '  Sub']);
  assert.equal(byLabel('Title').value, article.title);
  assert.equal(byLabel('DOI').value, '10.1000/abc.123');
  assert.equal(byLabel('Journal or venue').value, 'Journal of Tests');
  assert.equal(byLabel('Authors').value, 'Lovelace, Ada\nZotero Consortium');
  assert.equal(byLabel('A note with the 2 references').checked, true);
  assert.equal(byLabel('A link to the source URL').checked, true);
  assert.equal(byLabel('The original file').checked, false);
  for (const control of document.querySelectorAll('input,select,textarea')) assert.ok(control.labels.length > 0 || control.getAttribute('aria-label'), 'labelled: ' + control.id);
  for (const element of document.querySelectorAll('button')) assert.ok(element.textContent.trim(), 'buttons have text');
  assert.ok(document.querySelector('.zotero-panel [role="status"][aria-live="polite"]'), 'a live region reports progress');
  await act(async () => { setValue(byLabel('Collection'), 'SUBC2345'); });
  await act(async () => { setValue(byLabel('Tags'), 'tpe, to read'); });
  await act(async () => { button('Send to Zotero').click(); });
  await until(() => /Item ABCD2345 saved/.test(root.textContent), 'the send to finish');
  assert.equal(document.querySelector('.zotero-panel a[href="https://www.zotero.org/ada/items/ABCD2345"]')?.textContent, 'open it on zotero.org');
  const writes = calls.filter(call => call.url === `${API}/users/475425/items`);
  assert.equal(writes.length, 2, 'the item, then its note and link; no file upload unless chosen');
  const item = JSON.parse(writes[0].body)[0];
  assert.equal(item.itemType, 'journalArticle');
  assert.equal(item.title, article.title);
  assert.equal(item.DOI, '10.1000/abc.123');
  assert.deepEqual(item.collections, ['SUBC2345']);
  assert.deepEqual(item.tags, [{ tag: 'tpe' }, { tag: 'to read' }]);
  assert.deepEqual(item.creators, [{ creatorType: 'author', firstName: 'Ada', lastName: 'Lovelace' }, { creatorType: 'author', name: 'Zotero Consortium' }]);
  const children = JSON.parse(writes[1].body);
  assert.equal(children[0].itemType, 'note');
  assert.match(children[0].note, /10\.1000\/ref\.two/);
  assert.equal(children[1].url, 'https://example.org/a.pdf');
  assert.equal(document.querySelector('.zotero-panel [role="alert"]'), null, 'no error shown');
  assert.ok(calls.every(call => !call.url.includes(KEY) && !(typeof call.body === 'string' && call.body.includes(KEY))), 'the key appears in no URL or body');
  await act(async () => { button('Forget key').click(); });
  await until(() => !!byLabel('Zotero API key'), 'the key form again');
  assert.equal(window.sessionStorage.getItem('tpe-zotero-api-key'), null, 'the key is forgotten');
  await act(async () => { reactRoot.unmount(); });
});

test('Panel: a refused key is explained and nothing is stored', async () => {
  const reactRoot = createRoot(root);
  await act(async () => { reactRoot.render(createElement(Z.ZoteroPanel, { article, apiOptions: { fetch: async () => new Response('', { status: 403 }), sleep: async () => {} } })); });
  await until(() => byLabel('Zotero API key'), 'the key form');
  await act(async () => { setValue(byLabel('Zotero API key'), 'bad-key'); });
  await act(async () => { button('Connect').click(); });
  await until(() => document.querySelector('.zotero-panel [role="alert"]'), 'the error');
  assert.match(document.querySelector('.zotero-panel [role="alert"]').textContent, /refused the API key/);
  assert.equal(window.sessionStorage.getItem('tpe-zotero-api-key'), null);
  assert.equal(byLabel('Zotero API key').value, 'bad-key', 'the typed key stays for correction');
  await act(async () => { reactRoot.unmount(); });
});

let failed = 0;
for (const [name, run] of cases) {
  try { await run(); console.log('PASS', name); }
  catch (error) { failed++; console.log('FAIL', name); console.log(error && error.stack || error); }
}
console.log(failed ? `${failed} of ${cases.length} Zotero tests failed.` : `All ${cases.length} Zotero tests passed.`);
process.exit(failed ? 1 : 0);
