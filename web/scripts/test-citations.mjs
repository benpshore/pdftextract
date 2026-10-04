/** Real boundary/component modules; synthetic supplied data, no native/GROBID or resolver E2E. */
import assert from 'node:assert/strict';
import { createRequire, Module } from 'node:module';
import { fileURLToPath } from 'node:url';
const require = createRequire(import.meta.url);
const { JSDOM } = require('jsdom');
const dom = new JSDOM('<div id="root"></div>', { url: 'https://synthetic-citations.test/' });
for (const key of ['window', 'document', 'HTMLElement', 'Element', 'Node', 'HTMLInputElement', 'Event', 'MouseEvent']) globalThis[key] = dom.window[key];
Object.defineProperty(globalThis, 'navigator', { configurable: true, value: dom.window.navigator });
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const React = require('react'), { act } = React, { createRoot } = require('react-dom/client');
const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
const web = fileURLToPath(new URL('../', import.meta.url));
async function load(path) {
  const built = await build({ entryPoints: [web + path], tsconfig: web + 'tsconfig.json', bundle: true, write: false, platform: 'node', format: 'cjs', packages: 'external' });
  const compiled = new Module(web + path + '.synthetic.cjs');
  compiled.filename = web + path + '.synthetic.cjs'; compiled.paths = Module._nodeModulePaths(web);
  compiled._compile(built.outputFiles[0].text, compiled.filename);
  return compiled.exports;
}
const { validateCitations, citationUrl, citationCsvCell, citationCsvRows, serializeCitationCsv, filterCitationRows, CITATION_CSV_COLUMNS } = await load('lib/citations.ts');
const { CitationBrowser } = await load('components/citation-browser.tsx');
const record = { id: 'synthetic-document', sha256: 'a'.repeat(64), result_key: 'synthetic-document/results/revision-1', original_name: 'original.pdf', title: 'Synthetic source', kind: 'pdf', source_url: null, status: 'ready', engine: 'Synthetic fixture', bytes: 1, created_at: '2026-10-04T00:00:00Z' };
const makeBundle = () => ({
  schema: 'tpe.web-citations', version: 1,
  source: { document_id: record.id, sha256: record.sha256, result_key: record.result_key, original_name: record.original_name },
  provenance: { producer: 'native-grobid', native_version: 'synthetic-native', grobid_version: 'synthetic-grobid', generated_at: '2026-10-04T00:00:00Z' },
  document: { title: 'Supplied title', authors: ['Supplied author'], display_name: 'Supplied display name' },
  references: { state: 'ready', items: [
    { id: 'b1', label: '[7]', page: 3, title: '=SUM(1,2)', authors: ['Zoë, "A"', '李'], raw: 'Raw "quote", comma\nsecond line', doi: '10.0000/synthetic', url: 'https://example.org/synthetic', resolution: { status: 'resolved', providers: ['crossref'], note: 'Synthetic reported resolution only' } },
    { id: 'b2', title: '<script>synthetic</script>', authors: [], raw: '\t=HYPERLINK("synthetic")', url: 'javascript:alert(1)', resolution: { status: 'not-requested', providers: [] } },
  ] },
  mentions: { state: 'partial', reason: 'Synthetic partial mentions', items: [{ id: 'm1', text: '[1]', context: 'Synthetic context\nSecond line', page: 2, reference_ids: ['b1'] }] },
});
const checks = [], valid = bundle => validateCitations(bundle, record);
assert.equal(validateCitations(undefined, record).state, 'unavailable');
assert.equal(validateCitations(null, record).state, 'unavailable');
assert.equal(validateCitations(makeBundle()).state, 'unavailable');
assert.equal(valid(makeBundle()).state, 'available');
assert.deepEqual(valid(makeBundle()).binding, { hash: true, revision: true });
const noBinding = validateCitations(makeBundle(), { id: record.id, sha256: '', result_key: null });
assert.equal(noBinding.state, 'available'); assert.deepEqual(noBinding.binding, { hash: false, revision: false });
const mutations = [
  bundle => { bundle.version = 2; }, bundle => { bundle.schema = 'random'; },
  bundle => { bundle.source.document_id = 'other'; }, bundle => { bundle.source.sha256 = 'b'.repeat(64); },
  bundle => { delete bundle.source.sha256; }, bundle => { bundle.source.result_key = 'stale'; },
  bundle => { bundle.provenance.producer = 'browser'; }, bundle => { bundle.provenance.grobid_version = ''; },
  bundle => { bundle.provenance.generated_at = 'yesterday'; }, bundle => { bundle.references = []; },
  bundle => { bundle.references.items = {}; }, bundle => { bundle.references.items[0].authors = 'Author'; },
  bundle => { bundle.references.items[0].title = { html: '<b>unsafe</b>' }; }, bundle => { bundle.references.items[0].url = null; },
  bundle => { bundle.references.items[0].label = 7; }, bundle => { bundle.references.items[0].page = 0; }, bundle => { bundle.references.items[0].page = 2.5; },
  bundle => { bundle.references.items.push(bundle.references.items[0]); }, bundle => { bundle.references.items[0].unexpected = true; },
  bundle => { bundle.references.items[0].resolution.status = 'guessed'; }, bundle => { bundle.references.items[0].resolution.providers = []; },
  bundle => { bundle.references.items[0].resolution.providers = ['crossref', 'crossref']; },
  bundle => { bundle.references.items[1].resolution.providers = ['publisher']; },
  bundle => { bundle.mentions.items[0].reference_ids = ['absent']; }, bundle => { bundle.mentions.items[0].reference_ids = ['b1', 'b1']; },
  bundle => { bundle.mentions.items[0].page = 0; }, bundle => { bundle.mentions.items[0].page = 1.5; },
  bundle => { bundle.mentions.items[0].text = ''; }, bundle => { delete bundle.mentions.reason; },
];
for (const mutation of mutations) { const bundle = makeBundle(); mutation(bundle); assert.equal(valid(bundle).state, 'invalid', mutation.toString()); }
for (const value of [[], 'text', 4, true, {}, { schema: 'tpe.web-citations' }]) assert.equal(valid(value).state, 'invalid');
assert.equal(validateCitations(makeBundle(), { ...record, sha256: [] }).state, 'invalid');
assert.equal(validateCitations(makeBundle(), { ...record, result_key: {} }).state, 'invalid');
const noDeclaredRevision = makeBundle(); delete noDeclaredRevision.source.result_key;
assert.equal(valid(noDeclaredRevision).state, 'available'); assert.equal(valid(noDeclaredRevision).binding.revision, false);
checks.push('Unknown persisted JSON rejects wrong shapes, invalid relationships and known source/hash/declared-revision mismatch; omitted revision stays explicitly unchecked');

for (const producer of ['native', 'grobid']) {
  const bundle = makeBundle();
  bundle.provenance = { producer, [producer + '_version']: 'synthetic-version', generated_at: '2026-10-04T00:00:00Z' };
  bundle.mentions = { state: 'unavailable', reason: 'No in-text result supplied' };
  assert.equal(valid(bundle).state, 'available');
}
for (const state of ['unavailable', 'pending', 'failed']) {
  const bundle = makeBundle(); bundle.references = { state, reason: 'Synthetic reason' }; bundle.mentions.items[0].reference_ids = [];
  assert.equal(valid(bundle).state, 'available');
  assert.equal(citationCsvRows(bundle).length, 1);
}
const large = makeBundle(); large.references.items = Array.from({ length: 10_001 }, (_, i) => ({ ...large.references.items[0], id: 'b' + (i + 1) }));
large.references.items[0].raw = 'x'.repeat(100_001);
assert.equal(valid(large).state, 'available'); assert.equal(citationCsvRows(large).length, 10_002);
checks.push('Native-only/GROBID-only provenance stays truthful; mentions can independently exist without bibliography; no silent row/text caps');

for (const value of ['javascript:alert(1)', 'data:text/html,x', '//example.org', 'https://user:pass@example.org', ' https://example.org', 'https://example.org/\npath', 'file:///tmp/test']) assert.equal(citationUrl(value), undefined);
assert.equal(citationUrl('https://example.org/path'), 'https://example.org/path');
for (const value of ['=SUM(1,2)', '+cmd', '-1', '@foo', '  =SUM(1,2)', '\t=cmd', '\ntext', '\rtext', '\uFEFF=cmd']) assert.equal(citationCsvCell(value), "'" + value);
for (const value of ['plain', '李', 'Text\nsecond line', "'already escaped"]) assert.equal(citationCsvCell(value), value);
const canonical = makeBundle(), before = JSON.stringify(canonical), rows = citationCsvRows(canonical), csv = serializeCitationCsv(rows);
assert.equal(rows[0].cells[3], "'=SUM(1,2)"); assert.equal(rows[0].cells[2], '[7]'); assert.equal(rows[0].cells[11], '3'); assert.equal(csv[0], '\uFEFF');
assert(csv.includes('Zoë')); assert(csv.includes('李')); assert(csv.includes('Raw ""quote"", comma\nsecond line'));
assert.equal(JSON.stringify(canonical), before);
assert.equal(filterCitationRows(rows, 'zoë', 'reference').length, 1); assert.equal(filterCitationRows(rows, '', 'mention').length, 1);
function parseCsv(text) {
  const output = [], row = []; let cell = '', quoted = false;
  for (let i = text.startsWith('\uFEFF') ? 1 : 0; i < text.length; i++) {
    const char = text[i];
    if (char === '"') { if (quoted && text[i + 1] === '"') { cell += '"'; i++; } else quoted = !quoted; }
    else if (!quoted && char === ',') { row.push(cell); cell = ''; }
    else if (!quoted && char === '\r' && text[i + 1] === '\n') { row.push(cell); output.push([...row]); row.length = 0; cell = ''; i++; }
    else cell += char;
  }
  assert.equal(quoted, false); return output;
}
assert.deepEqual(parseCsv(csv), [CITATION_CSV_COLUMNS, ...rows.map(row => row.cells)]);
checks.push('CSV round-trips Unicode, multiline/quoted fields and structured arrays; formula-safe cells match inspection without modifying canonical values');

const root = createRoot(document.getElementById('root'));
const result = bibliography => ({ title: 'Synthetic source', text: 'Text', links: [{ url: 'https://ordinary-link.test/' }], warnings: [], engine: 'Synthetic', status: 'ready', ...(bibliography === undefined ? {} : { bibliography }) });
let currentBundle = makeBundle(), downloads = [];
const nativeCreate = URL.createObjectURL, nativeRevoke = URL.revokeObjectURL;
URL.createObjectURL = blob => { downloads.push(blob); return 'blob:synthetic-csv'; };
URL.revokeObjectURL = () => {};
const preventNavigation = event => { if (event.target instanceof dom.window.HTMLAnchorElement) event.preventDefault(); };
document.addEventListener('click', preventNavigation);
const render = async (bundle = currentBundle, selected = record, key = selected.id) => { await act(async () => root.render(React.createElement(CitationBrowser, { key, result: result(bundle), record: selected }))); };
const button = label => [...document.querySelectorAll('button')].find(element => element.textContent === label);
const click = async element => { assert(element); await act(async () => element.click()); };
const view = async value => { const select = document.querySelector('select[aria-label="Citation view"]'); assert(select); await act(async () => { select.value = value; select.dispatchEvent(new Event('change', { bubbles: true })); }); };
const filter = async value => { const input = document.querySelector('input[aria-label="Filter citations"]'); assert(input); await act(async () => { Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, 'value').set.call(input, value); input.dispatchEvent(new Event('input', { bubbles: true })); }); };
try {
  await act(async () => root.render(React.createElement(CitationBrowser, { result: result(undefined), record })));
  assert.match(document.body.textContent, /Bibliography unavailable/); assert.equal(document.querySelector('select'), null); assert.equal(button('Download CSV'), undefined);
  assert(!document.body.textContent.includes('ordinary-link.test'));
  const invalid = makeBundle(); invalid.source.result_key = 'stale'; await render(invalid);
  assert.match(document.body.textContent, /Supplied citations unavailable/); assert.equal(document.querySelector('select'), null);
  await render();
  assert.match(document.body.textContent, /Supplied provenance is unverified/);
  assert.equal(document.querySelectorAll('article').length, 2); assert.equal(document.querySelector('script'), null);
  assert.equal(document.querySelectorAll('a').length, 1); assert.equal(document.querySelector('a').getAttribute('rel'), 'noopener noreferrer');
  assert.match(document.body.textContent, /Original filename \(reported\)/);
  await view('mentions'); assert.equal(document.querySelectorAll('article').length, 1); assert.match(document.body.textContent, /In-text mentions are partial/); assert.match(document.body.textContent, /Reported reference IDs/);
  await view('csv'); assert.equal(document.querySelector('table'), null); assert.equal(document.querySelectorAll('article').length, 3);
  const firstCells = [...document.querySelectorAll('article:first-child dd')].map(cell => cell.textContent === 'Empty cell' ? '' : cell.textContent);
  assert.deepEqual(firstCells, rows[0].cells);
  await click(button('Download CSV')); assert.deepEqual(parseCsv(await downloads.at(-1).text()), [CITATION_CSV_COLUMNS, ...rows.map(row => row.cells)]);
  await filter('Zoë'); assert.equal(document.querySelectorAll('article').length, 1);
  await click(button('Download CSV')); assert.equal(parseCsv(await downloads.at(-1).text()).length, 2);
  await filter('absent'); assert.match(document.body.textContent, /No matching citations/); assert.equal(button('Download CSV').disabled, true);
  checks.push('Actual React controls distinguish absent/invalid/partial data, escape markup, allow only safe source URLs, filter and export exact inspected cells');

  const referenceOnly = makeBundle(); referenceOnly.mentions = { state: 'unavailable', reason: 'Native reference list only' };
  await render(referenceOnly, record, 'reference-only'); await view('mentions');
  assert.match(document.body.textContent, /In-text mentions unavailable/); assert.equal(button('Download CSV'), undefined); assert.equal(document.querySelector('article'), null);
  const empty = makeBundle(); empty.references = { state: 'ready', items: [] }; empty.mentions = { state: 'ready', items: [] };
  await render(empty, record, 'empty'); assert.match(document.body.textContent, /No references were reported/); assert.equal(button('Download CSV'), undefined);
  for (const state of ['pending', 'failed']) {
    const bundle = makeBundle(); bundle.references = { state, reason: '<script>Synthetic state</script>' }; bundle.mentions.items[0].reference_ids = [];
    await render(bundle, record, state); assert.match(document.body.textContent, state === 'pending' ? /References pending/ : /References extraction failed/); assert.equal(button('Download CSV'), undefined); assert.equal(document.querySelector('script'), null);
    await view('mentions'); assert.equal(document.querySelectorAll('article').length, 1);
  }
  const paged = makeBundle(); paged.references.items = Array.from({ length: 27 }, (_, i) => ({ ...paged.references.items[0], id: 'b' + (i + 1) }));
  await render(paged, record, 'paged'); assert.equal(document.querySelectorAll('article').length, 25);
  await click(button('Download CSV')); assert.equal(parseCsv(await downloads.at(-1).text()).length, 28);
  await click(button('Next citations')); assert.equal(document.querySelectorAll('article').length, 2); assert.equal(button('Next citations').disabled, true);
  await filter('absent');
  const other = { ...record, id: 'synthetic-other', result_key: 'synthetic-other/results/revision-2' }, otherBundle = makeBundle();
  otherBundle.source.document_id = other.id; otherBundle.source.result_key = other.result_key;
  await render(otherBundle, other); assert.equal(document.querySelector('input').value, ''); assert.equal(document.querySelector('select').value, 'references'); assert.equal(document.querySelectorAll('article').length, 2);
  await render(noDeclaredRevision); await filter('absent');
  await render(noDeclaredRevision, { ...record, result_key: 'synthetic-document/results/revision-2' });
  assert.equal(document.querySelector('input').value, ''); assert.match(document.body.textContent, /Not checked: no result revision supplied/);
  checks.push('Reference-only never implies mentions; empty/pending/failed stay distinct; full filtered CSV survives pagination; document and record-revision changes reset filters');
  console.log(JSON.stringify({ environment: 'Real TypeScript/React modules in jsdom; synthetic fixtures only, no scholarly service or resolver E2E', checks }, null, 2));
} finally {
  await act(async () => root.unmount()); document.removeEventListener('click', preventNavigation);
  URL.createObjectURL = nativeCreate; URL.revokeObjectURL = nativeRevoke; dom.window.close();
}
