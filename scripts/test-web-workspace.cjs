#!/usr/bin/env node
/**
 * Component integration regression for the actual web/app/workspace.tsx.
 *
 * Run from any directory after installing the locked web dependencies:
 *   node /path/to/pdftextract/scripts/test-web-workspace.cjs
 *
 * React, jsdom, DOMPurify, and the TypeScript compiler resolve from web/.
 * The DOM is emulated; extraction helpers, network, and persistence are mocked.
 * These checks validate queue/reader orchestration, not real browser layout,
 * PDF/OCR quality, IndexedDB durability, private storage, or a deployed Site.
 */
const assert = require('node:assert/strict');
const { createRequire } = require('node:module');
const Module = require('node:module');
const fs = require('node:fs');
const { File } = require('node:buffer');
const path = require('node:path');
const repositoryRoot = path.resolve(__dirname, '..');
const webRoot = path.join(repositoryRoot, 'web');
const req = createRequire(path.join(webRoot, 'package.json'));
const { JSDOM } = req('jsdom');
const dom = new JSDOM('<div id="root"></div>', { url: 'https://tpe.test/' });
for (const key of ['window', 'document', 'DOMParser', 'HTMLElement', 'HTMLTextAreaElement', 'Node', 'Event', 'MouseEvent', 'history', 'location'])
    global[key] = key === 'window' ? dom.window : dom.window[key];
Object.defineProperty(global, 'navigator', { value: dom.window.navigator, configurable: true });
global.File = File;
global.IS_REACT_ACT_ENVIRONMENT = true;
global.requestAnimationFrame = fn => { queueMicrotask(fn); return 1; };
let scroll = 0;
Object.defineProperty(window, 'scrollY', { get: () => scroll });
window.scrollTo = ({ top }) => { scroll = top; };
const React = req('react');
const { createRoot } = req('react-dom/client');
const { act } = React;
const ts = req('typescript');
const rows = new Map(), results = new Map(), saveCalls = [];
let failSecond = true, stored = null, releaseHold, failParse = false, holdOriginal = false, legacyOriginal = false;
const hold = new Promise(resolve => { releaseHold = resolve; });
const blank = (title, text) => ({ title, text, html: '<p>' + text + '</p><img src="https://tracking.invalid/pixel"><img src="/api/documents/doc1/assets/owned"><img src="/api/documents/other/assets/private"><style>body{display:none!important}</style><table background="https://tracking.invalid/background"><tbody><tr><td style="background:url(https://tracking.invalid/css)"></td></tr></tbody></table><input type="image" src="https://tracking.invalid/input">', links: [], warnings: [], engine: 'HTML parser', status: 'partial' });
// Exercise the real charset decoder through the component's saved-original path.
const uploadPath = path.join(webRoot, 'lib', 'upload-client.ts');
const uploadModule = new Module(uploadPath);
uploadModule.filename = uploadPath;
uploadModule._compile(ts.transpileModule(fs.readFileSync(uploadPath, 'utf8'), { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, uploadPath);
const helpers = {
    '@/components/ui/button': { Button: ({ asChild, children, variant, ...props }) => asChild ? React.cloneElement(children, props) : React.createElement('button', props, children) },
    '@/lib/clip': { clipHtml: (text, url, name) => { if (failParse === 'result')
            return { ...blank(name, ''), status: 'failed', warnings: ['Parser returned Failed'] }; if (failParse)
            throw new Error('Simulated parser failure'); return blank(name, 'HTML:' + text); }, parseFeed: (text) => blank('Feed', text), textDois: () => [], doiFrom: () => undefined, safeUrl: (value) => {
            try {
                const u = new URL(value);
                return /https?:/.test(u.protocol) ? u.href : null;
            }
            catch {
                return null;
            }
        } },
    '@/lib/imports': { expandUploads: async function* (files) {
            for (const file of files)
                yield { file, path: file.name };
        } },
    '@/lib/image-ocr': { recognizeImage: async () => { throw Error('Unexpected OCR'); } },
    '@/lib/office': { extractOffice: async () => { throw Error('Unexpected Office extraction'); } },
    '@/lib/article-assets': { retainArticleImages: async (record, result) => result, retainOfficeAssets: async (record, result) => result },
    '@/lib/workspace-storage': { readWorkspace: async () => stored, writeWorkspace: async (owner, snapshot) => { assert.equal(owner, 'owner-A'); stored = snapshot; } },
    '@/lib/upload-client': { readCaptureEvidence: uploadModule.exports.readCaptureEvidence, applyCaptureEvidence: uploadModule.exports.applyCaptureEvidence, captureWarning: uploadModule.exports.captureWarning, decodeSource: uploadModule.exports.decodeSource, uploadOriginal: async (file, { onProgress }) => {
            const text = await file.text(), id = 'doc' + (rows.size + 1), row = { id, title: file.name, kind: text.startsWith('<') ? 'html' : 'text', original_name: file.name, status: 'uploaded', engine: '', created_at: new Date().toISOString(), sha256: 'hash', bytes: file.size, source_url: null };
            rows.set(id, row);
            if (file.name === 'hold.txt')
                await hold;
            onProgress?.(1);
            return row;
        }, saveExtracted: async (record, result) => {
            saveCalls.push(record.id);
            if (record.original_name === 'second.txt' && failSecond) {
                failSecond = false;
                throw Error('Simulated interrupted result save');
            }
            results.set(record.id, result);
            rows.set(record.id, { ...record, title: result.title, status: result.status });
        }, captureSource: async () => { throw Error('Not exercised'); }, uploadAssetFile: async () => { throw Error('Not exercised'); } },
};
global.fetch = async (value, options = {}) => {
    const url = new URL(value, 'https://tpe.test');
    if (url.pathname === '/api/documents')
        return Response.json({ documents: [...rows.values()] });
    const id = url.pathname.split('/')[3];
    if (url.pathname.endsWith('/original')) {
        if (holdOriginal)
            await new Promise((resolve, reject) => options.signal.addEventListener('abort', () => reject(new DOMException('Cancelled', 'AbortError')), { once: true }));
        if (legacyOriginal)
            return new Response(Buffer.from('<html><meta charset="windows-1252"><body>Caf\xe9</body></html>', 'latin1'));
        return new Response('<html>First</html>');
    }
    return Response.json({ record: rows.get(id), result: results.get(id) || null });
};
global.Worker = class {
    constructor() { throw Error('HTML must never reach the PDF worker'); }
};
const source = path.join(webRoot, 'app', 'workspace.tsx');
const compiled = ts.transpileModule(fs.readFileSync(source, 'utf8'), { compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022, esModuleInterop: true } }).outputText;
const mod = new Module(source);
mod.filename = source;
mod.paths = Module._nodeModulePaths(webRoot);
mod.require = name => helpers[name] || req(name);
mod._compile(compiled, source);
const Workspace = mod.exports.default;
(async () => {
    const root = createRoot(document.getElementById('root'));
    await act(async () => { root.render(React.createElement(Workspace, { userId: 'owner-A' })); await new Promise(resolve => setTimeout(resolve, 20)); });
    // Boolean asserts with messages: assert.equal on a jsdom node would pretty-print the whole tree on failure.
    const absent = selector => assert(!document.querySelector(selector), selector + ' must be absent');
    const focused = (node, message) => assert(document.activeElement === node, message || 'unexpected focus');
    // Compact layout: the large empty-state hero is gone; one quiet hint points at Upload and the menu.
    absent('.empty-state');
    assert(!document.body.textContent.includes('Bring your reading here'));
    assert.match(document.querySelector('.empty-hint').textContent, /Upload.*Saved articles/);
    absent('.reader-heading');
    // Upload symbol: named, text-free, standard icon, tooltip is a decorative copy of the name.
    const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
    const key = name => new dom.window.KeyboardEvent('keydown', { key: name, bubbles: true });
    const upload = () => document.querySelector('button[aria-label="Upload"]');
    absent('[aria-label="Add files or a folder"]');
    assert.equal(upload().textContent, '');
    assert.equal(upload().querySelectorAll('svg').length, 1);
    assert.equal(upload().getAttribute('aria-expanded'), 'false');
    absent('.add-menu-options');
    const uploadTip = () => upload().closest('.tip-wrap');
    assert.equal(uploadTip().querySelector('.tip').textContent, 'Upload');
    assert.equal(uploadTip().querySelector('.tip').getAttribute('aria-hidden'), 'true');
    assert.equal(uploadTip().hasAttribute('data-tip-off'), false);
    assert([...document.querySelectorAll('input[type=file]')].every(input => input.tabIndex === -1 && input.getAttribute('aria-hidden') === 'true'), 'file inputs must be neither tab stops nor extra screen-reader buttons beside the Upload button');
    const fileInput = document.querySelector('input[type=file]');
    let pickerClicks = 0;
    fileInput.click = () => { pickerClicks++; };
    await act(async () => { upload().click(); });
    assert.equal(upload().getAttribute('aria-expanded'), 'true');
    assert.equal(document.querySelector('.menu-title').textContent, 'Upload');
    assert.deepEqual([...document.querySelectorAll('.add-menu-options button')].map(button => button.textContent), ['Add files', 'Add folder', 'Add photos']);
    assert.equal(uploadTip().hasAttribute('data-tip-off'), true, 'tooltip must yield to the options panel');
    await act(async () => { document.dispatchEvent(key('Escape')); });
    assert.equal(upload().getAttribute('aria-expanded'), 'false');
    focused(upload());
    assert.equal(uploadTip().hasAttribute('data-tip-off'), false, 'focus returned to Upload, so it is identified again');
    await act(async () => { document.dispatchEvent(key('Escape')); });
    assert.equal(uploadTip().hasAttribute('data-tip-off'), true, 'Escape dismisses the tooltip without moving focus');
    focused(upload());
    await act(async () => { upload().dispatchEvent(new dom.window.FocusEvent('focusin', { bubbles: true })); });
    assert.equal(uploadTip().hasAttribute('data-tip-off'), false, 'refocusing re-arms the tooltip');
    await act(async () => { upload().click(); });
    await act(async () => { document.body.dispatchEvent(new dom.window.Event('pointerdown', { bubbles: true })); });
    assert.equal(upload().getAttribute('aria-expanded'), 'false', 'outside press closes the options');
    await act(async () => { upload().click(); });
    await act(async () => { [...document.querySelectorAll('.add-menu-options button')].find(button => button.textContent === 'Add files').click(); });
    assert.equal(pickerClicks, 1);
    assert.equal(upload().getAttribute('aria-expanded'), 'false');
    focused(upload(), 'focus returns to Upload when the picker is launched');
    delete fileInput.click;
    Object.defineProperty(fileInput, 'files', { configurable: true, value: [new File(['<html>First</html>'], 'renamed.pdf'), new File(['Second'], 'second.txt')] });
    await act(async () => { fileInput.dispatchEvent(new Event('change', { bubbles: true })); await new Promise(resolve => setTimeout(resolve, 50)); });
    assert.equal(rows.size, 2);
    assert.equal(document.querySelectorAll('.reading img').length, 1);
    assert(document.querySelector('.reading img').src.endsWith('/api/documents/doc1/assets/owned'));
    assert.equal(document.querySelector('.reading style'), null);
    assert.equal(document.querySelector('.reading input'), null);
    assert.equal(document.querySelector('.reading table').hasAttribute('background'), false);
    assert.equal(document.querySelector('.reading [style]'), null);
    assert.equal(document.querySelector('.reading').textContent, 'HTML:First');
    assert.equal(new URL(location.href).searchParams.get('document'), 'doc1');
    const secondRow = [...document.querySelectorAll('.queue-item')].find(el => el.textContent.includes('second.txt'));
    assert.match(secondRow.textContent, /Save again/);
    assert.match(secondRow.textContent, /Simulated interrupted/);
    await act(async () => { secondRow.querySelector('.queue-open').click(); });
    assert.equal(document.querySelector('.reading').textContent, 'Second');
    scroll = 321;
    await act(async () => { document.querySelector('.queue-item .queue-open').click(); });
    assert.equal(document.querySelector('.reading').textContent, 'HTML:First');
    await act(async () => { [...secondRow.querySelectorAll('button')].find(button => button.textContent === 'Save again').click(); await new Promise(resolve => setTimeout(resolve, 30)); });
    assert.equal(saveCalls.at(-1), 'doc2');
    assert.equal(document.querySelector('.reading').textContent, 'HTML:First');
    assert.equal(results.get('doc2').text, 'Second');
    await act(async () => { history.back(); await new Promise(resolve => setTimeout(resolve, 30)); });
    assert.equal(new URL(location.href).searchParams.get('document'), 'doc2');
    assert.equal(document.querySelector('.reading').textContent, 'Second');
    assert.equal(scroll, 321);
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 300)); });
    assert.equal(stored.version, 1);
    assert.equal(stored.items.length, 2);
    assert(stored.items.every(item => item.source.type === 'stored'));
    assert(stored.items.every(item => !item.result));
    await act(async () => { root.unmount(); });
    const reopened = createRoot(document.getElementById('root'));
    await act(async () => { reopened.render(React.createElement(Workspace, { userId: 'owner-A' })); await new Promise(resolve => setTimeout(resolve, 40)); });
    assert.equal(document.querySelector('.reading').textContent, 'Second');
    assert.equal(document.querySelectorAll('.queue-item').length, 2);
    // Saved articles live behind the two-line menu; nothing is deleted and the queue is untouched.
    const menu = () => document.querySelector('button[aria-label="Saved articles"]');
    const dialog = () => document.querySelector('[role=dialog]'), layer = () => document.querySelector('.saved-layer');
    const inertParts = ['.app-header', '.workspace-grid', '.skip-link'];
    assert.equal(menu().getAttribute('aria-expanded'), 'false');
    assert.equal(menu().querySelector('path').getAttribute('d').split('M').length - 1, 2, 'two-line glyph');
    assert.equal(menu().closest('.tip-wrap').querySelector('.tip').textContent, 'Saved articles');
    assert.equal(layer().hidden, true);
    absent('.intake .document-list');
    absent('.intake .library');
    const savedBefore = rows.size, queueBefore = document.querySelectorAll('.queue-item').length;
    await act(async () => { menu().click(); });
    assert.equal(layer().hidden, false);
    assert.equal(dialog().getAttribute('aria-modal'), 'true');
    assert.equal(menu().getAttribute('aria-expanded'), 'true');
    assert.equal(document.activeElement.getAttribute('aria-label'), 'Close saved articles');
    inertParts.forEach(selector => assert(document.querySelector(selector).hasAttribute('inert'), selector + ' must be inert behind the dialog'));
    assert.deepEqual([...dialog().querySelectorAll('.document-item strong')].map(node => node.textContent).sort(), [...rows.values()].map(row => row.title).sort());
    assert.equal(dialog().querySelectorAll('ul.document-list > li > .document-item').length, rows.size, 'rows are list items');
    assert([...dialog().querySelectorAll('.document-item .help')].every(node => /\d{4}/.test(node.textContent)), 'every row says when it was saved');
    assert.equal(dialog().querySelectorAll('.document-item[aria-current=true]').length, 1);
    assert.match(dialog().querySelector('.document-item[aria-current=true]').textContent, /second\.txt/);
    await act(async () => { document.dispatchEvent(key('Escape')); });
    assert.equal(layer().hidden, true);
    focused(menu(), 'Escape returns focus to the menu button');
    inertParts.forEach(selector => assert.equal(document.querySelector(selector).hasAttribute('inert'), false));
    await act(async () => { menu().click(); });
    await act(async () => { document.querySelector('.saved-scrim').click(); });
    assert.equal(layer().hidden, true);
    await act(async () => { menu().click(); });
    await act(async () => { document.querySelector('button[aria-label="Close saved articles"]').click(); });
    assert.equal(layer().hidden, true);
    focused(menu());
    await act(async () => { menu().click(); });
    await act(async () => { [...dialog().querySelectorAll('.document-item')].find(button => button.textContent.includes('renamed.pdf')).click(); await sleep(40); });
    assert.equal(layer().hidden, true, 'choosing an article closes the menu');
    assert.equal(document.querySelector('.reading').textContent, 'HTML:First');
    assert.equal(new URL(location.href).searchParams.get('document'), 'doc1');
    focused(document.querySelector('.reader-heading h2'), 'focus lands on the chosen article');
    assert.equal(rows.size, savedBefore);
    assert.equal(document.querySelectorAll('.queue-item').length, queueBefore);
    await act(async () => { history.back(); await sleep(40); });
    assert.equal(document.querySelector('.reading').textContent, 'Second', 'Back still returns to the previous article');
    await act(async () => { menu().click(); });
    await act(async () => { history.forward(); await sleep(40); });
    assert.equal(layer().hidden, true, 'history navigation closes the menu instead of leaving it over a changed article');
    focused(menu(), 'history navigation that closes the menu returns focus to its button');
    assert.equal(document.querySelector('.reading').textContent, 'HTML:First');
    await act(async () => { history.back(); await sleep(40); });
    assert.equal(document.querySelector('.reading').textContent, 'Second');
    const pick = document.querySelector('input[type=file]');
    Object.defineProperty(pick, 'files', { configurable: true, value: [new File(['Hold'], 'hold.txt'), new File(['Cancel'], 'cancel.txt')] });
    await act(async () => { pick.dispatchEvent(new Event('change', { bubbles: true })); await new Promise(resolve => setTimeout(resolve, 20)); });
    await act(async () => { document.querySelector('button[aria-label="Cancel cancel.txt"]').click(); releaseHold(); await new Promise(resolve => setTimeout(resolve, 30)); });
    assert(![...rows.values()].some(row => row.original_name === 'cancel.txt'));
    assert.match([...document.querySelectorAll('.queue-item')].find(row => row.textContent.includes('cancel.txt')).textContent, /Cancelled/);
    await act(async () => { reopened.unmount(); });
    stored = { ...stored, items: [{ ...stored.items[0], phase: 'extracting' }] };
    history.replaceState({}, '', '/?queue=' + stored.items[0].id);
    const rowCount = rows.size;
    const recovered = createRoot(document.getElementById('root'));
    await act(async () => { recovered.render(React.createElement(Workspace, { userId: 'owner-A' })); await new Promise(resolve => setTimeout(resolve, 30)); });
    assert(document.querySelector('.phase-interrupted'));
    await act(async () => { [...document.querySelectorAll('.queue-item button')].find(button => button.textContent === 'Retry').click(); await new Promise(resolve => setTimeout(resolve, 30)); });
    assert.equal(rows.size, rowCount);
    assert(document.querySelector('.phase-saved'));
    const rereadButton = () => [...document.querySelectorAll('button')].find(button => button.textContent === 'Re-read original');
    legacyOriginal = true;
    const beforeReread = rows.size;
    await act(async () => { rereadButton().click(); await new Promise(resolve => setTimeout(resolve, 40)); });
    assert.equal(rows.size, beforeReread);
    assert.match(document.querySelector('.reading').textContent, /Café/);
    assert.doesNotMatch(document.querySelector('.reading').textContent, /\ufffd/);
    const validResult = results.get('doc1'), savesAfterSuccess = saveCalls.length;
    await act(async () => { document.querySelector('.queue-item .queue-open').click(); });
    assert.match(document.querySelector('.reading').textContent, /Café/);
    failParse = true;
    await act(async () => { rereadButton().click(); await new Promise(resolve => setTimeout(resolve, 35)); });
    assert.equal(saveCalls.length, savesAfterSuccess);
    assert.strictEqual(results.get('doc1'), validResult);
    assert.match(document.querySelector('.reading').textContent, /Café/);
    assert.match(document.querySelector('.queue-item:last-child').textContent, /Simulated parser failure/);
    failParse = 'result';
    await act(async () => { rereadButton().click(); await new Promise(resolve => setTimeout(resolve, 35)); });
    assert.equal(saveCalls.length, savesAfterSuccess);
    assert.strictEqual(results.get('doc1'), validResult);
    assert.match(document.querySelector('.reading').textContent, /Café/);
    assert.match(document.querySelector('.queue-item:last-child').textContent, /Parser returned Failed/);
    failParse = false;
    holdOriginal = true;
    await act(async () => { rereadButton().click(); await new Promise(resolve => setTimeout(resolve, 20)); });
    await act(async () => { document.querySelector('.queue-item:last-child button[aria-label^="Cancel "]').click(); await new Promise(resolve => setTimeout(resolve, 25)); });
    holdOriginal = false;
    assert.equal(saveCalls.length, savesAfterSuccess);
    assert.strictEqual(results.get('doc1'), validResult);
    assert.match(document.querySelector('.reading').textContent, /Café/);
    assert.match(document.querySelector('.queue-item:last-child').textContent, /Cancelled/);
    await act(async () => { recovered.unmount(); });
    // A first-import parser exception must preserve capture evidence in the generated
    // Failed result, not only in successful/partial extraction results. Reload uses
    // the persisted result and stored-source snapshot, without capturing again.
    for (const truncated of [true, false]) {
        const url = `https://capture.test/parser-exception-${truncated}`;
        const file = new File(['<html>Captured prefix</html>'], 'captured.html', { type: 'text/html' });
        const capture = { truncated, capturedBytes: file.size };
        let captureCalls = 0;
        helpers['@/lib/upload-client'].captureSource = async source => {
            assert.equal(source, url); captureCalls++;
            return { file, url, contentType: file.type, decodedSource: await file.text(), capture };
        };
        failParse = true; legacyOriginal = false;
        stored = { version: 1, items: [{ id: 'capture-exception', name: url, source: { type: 'url', url, feed: false }, phase: 'waiting', progress: null, message: 'Waiting' }], draft: { url: '', kind: '', paste: '', query: '' }, selection: { queueId: 'capture-exception' }, view: 'text', scroll: 0 };
        history.replaceState({}, '', '/?queue=capture-exception');
        const importing = createRoot(document.getElementById('root'));
        await act(async () => { importing.render(React.createElement(Workspace, { userId: 'owner-A' })); await sleep(30); });
        await act(async () => { [...document.querySelectorAll('.queue-item button')].find(button => button.textContent === 'Retry').click(); await sleep(40); });
        const failedId = saveCalls.at(-1), failed = results.get(failedId);
        assert.equal(failed.status, 'failed', 'parser exception must remain Failed');
        assert.match(failed.warnings.join(' '), /Simulated parser failure/);
        assert.deepEqual(failed.metadata?.sourceCapture, capture, 'generated Failed result retains source capture evidence');
        assert.equal(!!failed.metadata.truncated, truncated);
        assert.equal(!!document.querySelector('.capture-warning'), truncated);
        await act(async () => { importing.unmount(); });
        assert.equal(stored.items[0].source.type, 'stored');
        assert.deepEqual(stored.items[0].source.capture, capture);
        assert.equal(stored.items[0].result, undefined, 'reload must fetch the durable result');
        const reloaded = createRoot(document.getElementById('root'));
        await act(async () => { reloaded.render(React.createElement(Workspace, { userId: 'owner-A' })); await sleep(40); });
        assert.match(document.querySelector('#reader .notice.error').textContent, /Simulated parser failure/);
        assert.equal(!!document.querySelector('.capture-warning'), truncated, 'capture warning survives reload');
        assert.equal(results.get(failedId).status, 'failed');
        assert.equal(captureCalls, 1, 'reload does not re-fetch the source');
        await act(async () => { reloaded.unmount(); });
    }
    console.log(JSON.stringify({ multi_file_independent_failure: true, content_dispatch_over_extension: true, stable_selection: true, retry_owner_preserved: true, back_reader_and_scroll: true, indexeddb_snapshot_drops_saved_files: true, remount_reader_restored: true, private_asset_only_rendering: true, queued_cancel_skips_upload: true, interrupted_retry_reuses_owned_original: true, reader_css_and_active_inputs_removed: true, reread_reuses_owned_original: true, hero_removed_compact_hint: true, upload_symbol_named_tooltip_options_focus: true, saved_articles_menu_open_close_select_focus: true, saved_html_charset_recovered: true, failed_reread_keeps_valid_result: true, failed_status_reread_keeps_valid_result: true, cancelled_reread_keeps_valid_result: true, parser_exception_retains_capture_evidence_after_reload: true, complete_capture_exception_has_no_truncation_warning: true }));
})().catch(error => { console.error(error); process.exitCode = 1; });
