// Article extraction regressions on synthetic fixtures. No network, no page scripts executed.
// Run from web/: node lib/article-extract/tests/run.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { JSDOM, VirtualConsole } from 'jsdom';

const require = createRequire(import.meta.url);
const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
const here = dirname(fileURLToPath(import.meta.url));
const web = resolve(here, '..', '..', '..');
const window = new JSDOM('', { url: 'https://capture.test/', virtualConsole: new VirtualConsole() }).window;
for (const key of ['window', 'document', 'DOMParser', 'Node', 'Element', 'HTMLElement']) globalThis[key] = window[key];
async function load(entry) {
  const bundle = await build({ entryPoints: [resolve(web, entry)], bundle: true, write: false, format: 'esm', platform: 'node' });
  return import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
}
const article = await load('lib/article-extract/index.ts');
const { extractArticle, extractArticleChain, nextPageUrl, normalizeDate, repairHtml, scoreBlocks, parseHtml, sameSite, DEFAULT_LIMITS } = article;
const fixture = name => readFileSync(resolve(here, '..', 'fixtures', name), 'utf8');
const absent = (result, markers) => { const blob = JSON.stringify(result); for (const marker of markers) assert.ok(!blob.includes(marker), `leaked ${marker}`); };

const cases = [
  ['News article: metadata, boilerplate removal and Markdown structure', () => {
    const result = extractArticle(fixture('news-article.html'), 'https://www.tidewatch.test/news/harbour-sensors-fuel?utm=share');
    assert.equal(result.title, 'Harbour sensors cut night-time fuel use by a third');
    assert.deepEqual(result.authors.map(a => a.name), ['Mira Okonkwo', 'Tom Ashdown']);
    assert.equal(result.authors[0].url, 'https://www.tidewatch.test/staff/mira');
    assert.equal(result.byline, 'By Mira Okonkwo, Tom Ashdown');
    assert.equal(result.published, '2026-03-14T08:30:00+01:00');
    assert.equal(result.modified, '2026-03-15T10:05:00Z');
    assert.equal(result.canonicalUrl, 'https://www.tidewatch.test/news/harbour-sensors-fuel');
    assert.equal(result.siteName, 'Tidewatch Gazette');
    assert.equal(result.language, 'en-GB');
    assert.equal(result.description, 'A pilot on the east quay replaced timed pumps with level sensors.');
    assert.equal(result.amp.isAmp, false);
    assert.equal(result.amp.ampUrl, 'https://www.tidewatch.test/amp/news/harbour-sensors-fuel');
    assert.deepEqual(result.feeds, [{ type: 'application/rss+xml', url: 'https://www.tidewatch.test/feed.xml' }]);
    assert.equal(result.paywall.detected, false);
    assert.equal(result.selection, 'readability');
    absent(result, ['LDJSON_BODY_SECRET', 'HYDRATION_SECRET', 'STYLE_SECRET', 'COOKIE_BANNER_TEXT', 'NAV_NEWS', 'NAV_ACCOUNT', 'BREADCRUMB_TEXT', 'AD_SLOT_TEXT', 'SHARE_TEXT', 'RELATED_TEXT', 'COMMENT_TEXT', 'SIDEBAR_TEXT', 'FOOTER_TEXT']);
    assert.match(result.markdown, /^# Harbour sensors cut night-time fuel use by a third/m);
    assert.match(result.markdown, /^## What the logs show/m);
    assert.match(result.markdown, /^- +Pump run time fell from 240 minutes/m);
    assert.match(result.markdown, /^1\. +Fit sensors to the remaining twelve pontoons/m);
    assert.match(result.markdown, /^> We expected a small saving/m);
    assert.match(result.markdown, /!\[A level sensor clamped to a pontoon\]\(https:\/\/www\.tidewatch\.test\/img\/east-quay-sensor-large\.jpg\)/);
    assert.match(result.markdown, /\*The sensor tube on pontoon C, photographed during the January storms\.\*/);
    assert.match(result.markdown, /\| Month \| Run minutes per night \|/);
    assert.match(result.text, /^- Pump run time fell/m);
    assert.match(result.text, /^1\. Fit sensors/m);
    assert.match(result.text, /^> We expected/m);
    assert.match(result.text, /The sensor tube on pontoon C/);
    assert.deepEqual(result.tables, [[['Month', 'Run minutes per night'], ['November', '236'], ['February', '151']]]);
    assert.deepEqual(result.images.map(image => image.url), ['https://www.tidewatch.test/img/east-quay-sensor-large.jpg']);
    assert.equal(result.images[0].caption, 'The sensor tube on pontoon C, photographed during the January storms.');
    assert.deepEqual(result.headings.map(heading => heading.title), ['What the logs show', 'Next steps for the west quay']);
    assert.deepEqual(result.structuredData[0].author, ['Mira Okonkwo', 'Tom Ashdown']);
    assert.equal(result.scholarly, null);
    assert.equal(result.nextPageUrl, null);
    assert.ok(result.warnings.some(warning => /without running scripts/.test(warning)));
  }],
  ['Scholarly article: citation_*, Dublin Core, PRISM, JSON-LD and identifier links', () => {
    const result = extractArticle(fixture('scholarly.html'), 'https://journal.test/article/10.9999/jhi.2026.0042');
    assert.equal(result.title, 'Seasonal drift in pontoon sensor calibration');
    assert.deepEqual(result.authors.map(a => a.name), ['Okonkwo, Mira', 'Ashdown, Tom', 'Ríos, Beatriz']);
    assert.equal(result.published, '2026-02-09');
    assert.equal(result.modified, '2026-02-20');
    assert.equal(result.language, 'en');
    assert.equal(result.siteName, 'Harbour Press');
    const s = result.scholarly;
    assert.equal(s.doi, '10.9999/jhi.2026.0042');
    assert.equal(s.pmid, '41234567');
    assert.equal(s.pmcid, 'PMC7654321');
    assert.equal(s.journal, 'Journal of Harbour Instrumentation');
    assert.equal(s.volume, '12');
    assert.equal(s.issue, '1');
    assert.equal(s.firstPage, '41');
    assert.equal(s.lastPage, '58');
    assert.equal(s.issn, '2049-0001');
    assert.equal(s.publisher, 'Harbour Press');
    assert.equal(s.pdfUrl, 'https://journal.test/pdf/jhi.2026.0042.pdf');
    assert.equal(s.abstract, 'We measured drift in twelve level sensors over one winter.');
    assert.deepEqual(s.keywords, ['calibration', 'pontoon', 'level sensor']);
    assert.equal(s.type, 'ScholarlyArticle');
    assert.ok(result.links.some(link => /DOI/.test(link.kind) && link.doi === '10.9999/jhi.2026.0042'));
    assert.ok(result.links.some(link => link.doi === '10.9999/hn.2024.0007'));
    assert.ok(result.links.some(link => /PubMed|PMID/.test(link.kind) && /41234567/.test(link.url)));
    assert.match(result.markdown, /^## Methods/m);
    assert.match(result.text, /Mean offset \(mm\)/);
    assert.equal(result.tables.length, 1);
    absent(result, ['NAV_ISSUES', 'FOOTER_TEXT']);
  }],
  ['AMP page: amp-img becomes img, AMP chrome is removed, canonical and language are kept', () => {
    const result = extractArticle(fixture('amp.html'), 'https://www.tidewatch.test/amp/news/harbour-sensors-fuel');
    assert.equal(result.amp.isAmp, true);
    assert.equal(result.canonicalUrl, 'https://www.tidewatch.test/news/harbour-sensors-fuel');
    assert.equal(result.language, 'de');
    assert.equal(result.title, 'Hafenbericht: Sensoren sparen Treibstoff');
    assert.deepEqual(result.images.map(image => image.url), ['https://www.tidewatch.test/img/sensor-large.jpg', 'https://www.tidewatch.test/img/sensor-wide.jpg']);
    assert.equal(result.images[0].caption, 'Der Sensor am Ponton C.');
    absent(result, ['ANALYTICS_SECRET', 'SIDEBAR_LINK', 'NOTICE_TEXT', 'AD_SLOT_SECRET', 'AD_PLACEHOLDER_TEXT', 'FOOTER_TEXT', 'amp-img', 'amp-ad']);
    assert.match(result.text, /Füllstandsensoren am Ostkai/);
  }],
  ['Paywall preview: public text only, gated sections and prompts dropped, never bypassed', () => {
    const result = extractArticle(fixture('paywall.html'), 'https://www.tidewatch.test/long-reads/pump-room');
    assert.equal(result.paywall.detected, true);
    assert.ok(result.paywall.evidence.length >= 3, JSON.stringify(result.paywall));
    assert.match(result.text, /public preview of a longer piece/);
    assert.match(result.text, /short teaser paragraph/);
    absent(result, ['PAID_BODY_IN_JSONLD', 'PAID_SECTION_TEXT', 'AMP_ACCESS_SECRET', 'PAYWALL_PROMPT_TEXT']);
    assert.ok(result.warnings.some(warning => /paywall/i.test(warning) && /no bypass/.test(warning)));
    assert.equal(result.published, '2026-04-02');
  }],
  ['Multi-page: rel=next on the same site is followed, merged and renumbered; cross-site next is ignored', async () => {
    const pages = {
      'https://www.tidewatch.test/long-reads/long-tide': 'multipage-1.html',
      'https://www.tidewatch.test/long-reads/long-tide?page=2': 'multipage-2.html',
      'https://www.tidewatch.test/long-reads/long-tide?page=3': 'multipage-3.html',
    };
    const requested = [];
    const loader = async url => { requested.push(url); if (!pages[url]) throw new Error(`unexpected ${url}`); return fixture(pages[url]); };
    const first = extractArticle(fixture('multipage-1.html'), 'https://www.tidewatch.test/long-reads/long-tide');
    assert.equal(first.nextPageUrl, 'https://www.tidewatch.test/long-reads/long-tide?page=2');
    const result = await extractArticleChain(fixture('multipage-1.html'), 'https://www.tidewatch.test/long-reads/long-tide', loader);
    assert.deepEqual(requested, ['https://www.tidewatch.test/long-reads/long-tide?page=2', 'https://www.tidewatch.test/long-reads/long-tide?page=3']);
    assert.deepEqual(result.pages, Object.keys(pages));
    assert.equal(result.nextPageUrl, null, 'cross-site next must not be followed or reported');
    assert.equal(result.title, 'The long tide');
    assert.equal(result.published, '2026-05-01');
    assert.deepEqual(result.authors.map(a => a.name), ['Tom Ashdown']);
    assert.match(result.text, /Page one opens/);
    assert.match(result.text, /Page two follows/);
    assert.match(result.text, /Page three closes/);
    assert.equal((result.markdown.match(/^# The long tide/gm) || []).length, 1, 'repeated page titles are dropped');
    assert.deepEqual(result.images.map(image => image.id), ['image-1', 'image-2']);
    assert.deepEqual(result.images.map(image => image.caption), ['The drums on the morning of the count.', 'Pontoon C in the January storm.']);
    assert.equal(new Set(result.headings.map(heading => heading.id)).size, result.headings.length);
    assert.match(result.html, /data-image-id="image-2"/);
    assert.match(result.html, /data-article-page="2"/);
    absent(result, ['FOOTER_TEXT']);
    const limited = await extractArticleChain(fixture('multipage-1.html'), 'https://www.tidewatch.test/long-reads/long-tide', loader, { maxPages: 2 });
    assert.equal(limited.pages.length, 2);
    assert.ok(limited.warnings.some(warning => /page limit of 2/.test(warning)));
    const failing = await extractArticleChain(fixture('multipage-1.html'), 'https://www.tidewatch.test/long-reads/long-tide', async () => { throw new Error('offline'); });
    assert.equal(failing.pages.length, 1);
    assert.ok(failing.warnings.some(warning => /could not be loaded: offline/.test(warning)));
    let clock = 0;
    const slow = await extractArticleChain(fixture('multipage-1.html'), 'https://www.tidewatch.test/long-reads/long-tide', async url => { clock += 5000; return loader(url); }, { deadlineMs: 4000, now: () => clock });
    assert.equal(slow.pages.length, 2);
    assert.ok(slow.warnings.some(warning => /time budget/.test(warning)));
    const looping = await extractArticleChain(fixture('multipage-1.html').replace('/long-reads/long-tide?page=2', '/long-reads/long-tide?page=2#top'), 'https://www.tidewatch.test/long-reads/long-tide', async () => fixture('multipage-1.html'));
    assert.equal(looping.pages.length, 1);
    assert.ok(looping.warnings.some(warning => /repeated earlier content|loops back/.test(warning)), JSON.stringify(looping.warnings));
    const aborted = new AbortController(); aborted.abort();
    await assert.rejects(extractArticleChain(fixture('multipage-1.html'), 'https://www.tidewatch.test/long-reads/long-tide', loader, { signal: aborted.signal }));
    const doc = parseHtml('<a rel="next" href="https://evil.test/next">Next</a><a class="next" href="/story?page=2">Next</a>');
    assert.equal(nextPageUrl(doc, 'https://news.example.test/story', 'https://news.example.test/story'), 'https://news.example.test/story?page=2');
    assert.ok(sameSite('www.example.test', 'example.test') && sameSite('news.example.test', 'example.test') && !sameSite('example.test', 'example.net'));
  }],
  ['Lazy images: data-src, data-srcset, picture sources, noscript fallbacks and pixel removal', () => {
    const result = extractArticle(fixture('lazy-images.html'), 'https://gallery.test/notes');
    assert.deepEqual(result.images.map(image => image.url), [
      'https://gallery.test/img/one-real.jpg',
      'https://gallery.test/img/two-large.jpg',
      'https://gallery.test/img/three-webp-large.webp',
      'https://gallery.test/img/four-noscript.jpg',
      'https://gallery.test/img/five-plain.jpg',
      'https://gallery.test/img/six-lazy.jpg',
    ]);
    assert.deepEqual(result.images.slice(0, 3).map(image => image.caption), ['Caption one.', 'Caption two.', 'Caption three.']);
    assert.ok(!/tracking-pixel|spacer|blank\.gif|data:image|srcset=/.test(result.html));
    assert.equal(result.images[4].width, '800');
  }],
  ['Garbled HTML: NULs, stray tags, unterminated comment and truncated script are repaired and reported', () => {
    const source = fixture('garbled.html').replace('{{NUL}}', '\u0000');
    const result = extractArticle(source, 'https://garbled.test/page');
    assert.equal(result.title, 'Garbled but readable');
    assert.match(result.text, /TEXT_AFTER_COMMENT/);
    assert.match(result.text, /Mis-nested bold and italic text/);
    assert.match(result.text, /Unclosed list item two/);
    assert.match(result.text, /NULbyte/);
    assert.ok(result.repairs.some(repair => /comment/.test(repair)) && result.repairs.some(repair => /<script>/.test(repair)) && result.repairs.some(repair => /NUL/.test(repair)), JSON.stringify(result.repairs));
    assert.ok(result.warnings.some(warning => /U\+FFFD/.test(warning)));
    assert.ok(result.warnings.some(warning => /incomplete or garbled/.test(warning)));
    assert.ok(!result.text.includes('the page was cut off'));
    const bare = extractArticle('Just a sentence with no markup at all, long enough to be noticed by the reader.', 'https://garbled.test/bare');
    assert.match(bare.text, /no markup at all/);
    assert.equal(extractArticle('', 'https://garbled.test/empty').text, '');
    const mojibake = extractArticle(`<article><h1>Encoding</h1><p>${'The rÃ©sumÃ© of the cafÃ© â€” naÃ¯ve again. '.repeat(10)}</p></article>`, 'https://garbled.test/mojibake');
    assert.ok(mojibake.warnings.some(warning => /mojibake/.test(warning)));
    assert.deepEqual(repairHtml('<p>a</p><!-- open <p>b</p>').repairs, ['Neutralised an unterminated HTML comment.']);
    assert.equal(repairHtml('<p>a</p><!-- closed --><p>b</p>').repairs.length, 0);
  }],
  ['Determinism and limits: identical input gives identical output; size caps truncate with warnings', () => {
    const source = fixture('news-article.html');
    const one = extractArticle(source, 'https://www.tidewatch.test/news/harbour-sensors-fuel');
    const two = extractArticle(source, 'https://www.tidewatch.test/news/harbour-sensors-fuel');
    assert.deepEqual(one, two);
    assert.equal(JSON.stringify(one), JSON.stringify(two));
    assert.ok(!('capturedAt' in one));
    const capped = extractArticle(fixture('lazy-images.html'), 'https://gallery.test/notes', { limits: { maxImages: 2, maxLinks: 0 } });
    assert.equal(capped.images.length, 2);
    assert.ok(capped.warnings.some(warning => /4 image\(s\) beyond the limit of 2/.test(warning)));
    assert.ok(!/data-image-id="image-3"/.test(capped.html));
    const big = extractArticle(`${source}<p>${'x'.repeat(100)}</p>`, 'https://www.tidewatch.test/news/harbour-sensors-fuel', { limits: { maxInputChars: source.length - 500 } });
    assert.equal(big.truncated, true);
    assert.ok(big.warnings.some(warning => /cut at/.test(warning)));
    assert.match(big.text, /east quay pilot/);
    assert.ok(DEFAULT_LIMITS.maxInputChars > 1024 * 1024);
    const huge = extractArticle(source.replace('</head>', `<script>${'y'.repeat(3 * 1024 * 1024)}</script></head>`), 'https://www.tidewatch.test/news/harbour-sensors-fuel');
    assert.equal(huge.text, one.text);
  }],
  ['Block scorer: the dense prose container outranks link-heavy chrome when no landmarks exist', () => {
    const prose = 'The scorer prefers containers whose paragraphs carry commas, length and few links, exactly like this one does. ';
    const doc = parseHtml(`<div id="wrap"><div class="sidebar"><ul>${'<li><a href="/x">Link item</a></li>'.repeat(20)}</ul></div><div class="post-body"><p>${prose.repeat(3)}</p><p>${prose.repeat(3)}</p><p>${prose.repeat(2)}</p></div><div class="comments"><p>${prose.repeat(2)} <a href="/c">reply</a></p></div></div>`);
    const ranked = scoreBlocks(doc);
    assert.equal(ranked[0].element.className, 'post-body');
    assert.ok(ranked.find(block => block.element.className === 'comments').score < ranked[0].score);
    const result = extractArticle(`<title>Plain</title><div class="sidebar"><ul>${'<li><a href="/x">Link item</a></li>'.repeat(20)}</ul></div><div class="post-body"><p>${prose.repeat(3)}</p><p>${prose.repeat(3)}</p></div>`, 'https://plain.test/');
    assert.match(result.text, /The scorer prefers containers/);
    assert.ok(!/Link item/.test(result.text));
  }],
  ['Date normalisation keeps offsets and never applies the local time zone', () => {
    assert.equal(normalizeDate('2026-03-14T08:30:00+0100'), '2026-03-14T08:30:00+01:00');
    assert.equal(normalizeDate('2026-03-14 08:30'), '2026-03-14T08:30:00');
    assert.equal(normalizeDate('2026/3/4'), '2026-03-04');
    assert.equal(normalizeDate('March 4, 2026'), '2026-03-04');
    assert.equal(normalizeDate('Wednesday, 4 March 2026'), '2026-03-04');
    assert.equal(normalizeDate('Sat, 14 Mar 2026 07:30:00 GMT'), '2026-03-14T07:30:00Z');
    assert.equal(normalizeDate('2026'), '2026');
    assert.equal(normalizeDate('spring 2026'), 'spring 2026');
    assert.equal(normalizeDate(''), null);
  }],
];

let failed = 0;
for (const [name, check] of cases) {
  try { await check(); console.log(`PASS ${name}`); }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack || error.message}`); }
}
console.log(`${cases.length - failed}/${cases.length} article extraction cases passed.`);
if (failed) process.exitCode = 1;
