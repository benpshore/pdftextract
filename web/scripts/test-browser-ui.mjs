/** Actual browser regression for the compact layout: Upload symbol and tooltip, no hero, saved-articles dialog.
 * Not part of CI. Needs a running local app and Playwright Chromium (PLAYWRIGHT_MODULE may point to a provisioned module):
 *   web/.openai/hosting.json present, `pnpm dev` running (mock sign-in), then `node scripts/test-browser-ui.mjs`.
 * TPE_BASE_URL overrides http://127.0.0.1:5173. The local D1 schema is applied here when it is missing.
 * Chromium only: it says nothing about Safari, VoiceOver or a real iPhone; touch is device emulation.
 */
import { readdirSync, readFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const { chromium, devices } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const BASE = process.env.TPE_BASE_URL || 'http://127.0.0.1:5173';
const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const results = [];
const check = (profile, name, ok, detail) => { results.push({ profile, name, ok: !!ok }); console.log(ok ? 'PASS' : 'FAIL', profile, '·', name, ok || detail === undefined ? '' : '· ' + JSON.stringify(detail).slice(0, 400)); };
const overlaps = (a, b) => !(a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y);
const shown = (page, selector) => page.evaluate(s => { const e = document.querySelector(s); return !!e && getComputedStyle(e).display !== 'none' && e.getBoundingClientRect().width > 0; }, selector);
const activeLabel = page => page.evaluate(() => document.activeElement?.getAttribute('aria-label') || document.activeElement?.tagName);
const runId = Date.now().toString(36);

async function ensureSchema() {
  const probe = await fetch(BASE + '/api/documents?q=', { headers: { cookie: '__sites_local_auth=1' } }).catch(() => null);
  if (!probe) throw new Error('No app at ' + BASE + ': start `pnpm dev` first.');
  if (!(await probe.text()).includes('no such table')) return;
  const { DatabaseSync } = await import('node:sqlite');
  const dir = join(web, '.wrangler/state/v3/d1/miniflare-D1DatabaseObject');
  const file = readdirSync(dir).find(name => name.endsWith('.sqlite') && name !== 'metadata.sqlite');
  const db = new DatabaseSync(join(dir, file));
  for (const sql of readdirSync(join(web, 'drizzle')).filter(name => name.endsWith('.sql')).sort())
    db.exec(readFileSync(join(web, 'drizzle', sql), 'utf8').replaceAll('--> statement-breakpoint', ''));
  db.close();
}
async function boot(browser, options) {
  const context = await browser.newContext({ ...options, baseURL: BASE });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto('/signin-with-chatgpt?return_to=%2F');
  await page.waitForSelector('textarea#source-paste', { timeout: 60000 });
  for (let attempt = 0; attempt < 40; attempt++) { // dev hydration can lag: wait until the controlled textarea enables Send
    await page.fill('textarea#source-paste', 'x');
    if (await page.locator('button[aria-label="Import pasted source"]').isEnabled()) break;
    await page.waitForTimeout(250);
  }
  await page.fill('textarea#source-paste', '');
  return { context, page, errors };
}
async function addFile(page, name, body) {
  await page.setInputFiles('input[aria-label="Choose source files"]', { name, mimeType: 'text/plain', buffer: Buffer.from(body) });
  await page.waitForSelector(`.queue-item:has-text("${name}") .phase-saved`, { timeout: 60000 });
}
const openUpload = page => page.locator('button[aria-label="Upload"]').click();

async function wideDesktop(browser, width, height) {
  const P = `desktop-${width}x${height}-empty-queue`;
  const { context, page, errors } = await boot(browser, { viewport: { width, height } });
  const upload = page.locator('button[aria-label="Upload"]');
  await openUpload(page);
  await page.waitForTimeout(150);
  const last = await page.evaluate(() => { const buttons = [...document.querySelectorAll('.add-menu-options button')], b = buttons.at(-1), r = b.getBoundingClientRect(), hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2); return { label: b.textContent, hitsItself: b === hit || b.contains(hit), intakeScroll: document.querySelector('.intake').scrollTop }; });
  check(P, 'last option is fully hit-testable (not clipped by the sticky sidebar)', last.hitsItself, last);
  const box = await upload.boundingBox();
  check(P, 'composer was not scrolled away: Upload stays in view and the sidebar did not scroll', box.y >= 0 && box.y + box.height <= height && last.intakeScroll === 0, { box, scroll: last.intakeScroll });
  const [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.getByRole('button', { name: 'Add photos' }).click()]);
  check(P, 'centre click on the last option opens the picker', chooser.isMultiple());
  await chooser.setFiles([]);
  await addFile(page, `long-${runId}-${width}.txt`, Array.from({ length: 120 }, (_, i) => `Paragraph ${i}. ` + 'Lorem ipsum dolor sit amet. '.repeat(8)).join('\n\n'));
  await page.mouse.wheel(0, 2500);
  await page.waitForTimeout(250);
  const menu = await page.locator('button[aria-label="Saved articles"]').boundingBox();
  check(P, 'Saved articles button stays reachable while reading deep into an article', menu.y >= 0 && menu.y + menu.height <= height, menu);
  const upBox = await upload.boundingBox();
  check(P, 'sticky sidebar clears the sticky header', upBox.y >= 59, upBox);
  check(P, 'no page errors', errors.length === 0, errors);
  await context.close();
}

async function desktop(browser) {
  const P = 'desktop-1440x900';
  const { context, page, errors } = await boot(browser, { viewport: { width: 1440, height: 900 } });
  const up = page.locator('button[aria-label="Upload"]');
  check(P, 'hero removed; one hint names Upload and the Saved articles button', !(await page.locator('main').innerText()).includes('Bring your reading here') && /Upload.*Saved articles/.test(await page.locator('.empty-hint').innerText()));
  check(P, 'Upload: named, icon only, collapsed', (await up.innerText()) === '' && (await up.locator('svg').count()) === 1 && (await up.getAttribute('aria-expanded')) === 'false');
  const upBox = await up.boundingBox();
  check(P, 'Upload target >= 44px', upBox.width >= 44 && upBox.height >= 44, upBox);
  check(P, 'tooltip hidden at rest', !(await shown(page, '.composer .tip')));
  await up.hover();
  await page.waitForTimeout(100);
  const tip = await page.locator('.composer .tip').boundingBox();
  check(P, 'hover shows "Upload" beside the control without covering it', (await shown(page, '.composer .tip')) && (await page.locator('.composer .tip').innerText()) === 'Upload' && !overlaps(tip, upBox), { tip, upBox });
  await page.mouse.move(upBox.x + upBox.width + 8, upBox.y + upBox.height / 2, { steps: 12 });
  check(P, 'tooltip stays while the pointer travels onto it (hoverable)', await shown(page, '.composer .tip'));
  await page.keyboard.press('Escape');
  check(P, 'Escape dismisses the tooltip with the pointer still over it', !(await shown(page, '.composer .tip')));
  await page.mouse.move(700, 600);
  await page.focus('textarea#source-paste');
  await page.keyboard.press('Tab');
  check(P, 'Tab from the textarea lands on Upload; file inputs add no tab stops', (await activeLabel(page)) === 'Upload');
  check(P, 'keyboard focus shows the tooltip and a >= 2px focus ring', (await shown(page, '.composer .tip')) && (await page.evaluate(() => parseFloat(getComputedStyle(document.activeElement).outlineWidth) >= 2)));
  await page.keyboard.press('Enter');
  check(P, 'Enter opens the labelled options; the tooltip yields', (await up.getAttribute('aria-expanded')) === 'true' && !(await shown(page, '.composer .tip')) && (await page.locator('.menu-title').innerText()) === 'Upload');
  check(P, 'options: files, folder, photos, each >= 44px', JSON.stringify((await page.locator('.add-menu-options button').allInnerTexts()).map(t => t.trim())) === '["Add files","Add folder","Add photos"]' && (await page.locator('.add-menu-options button').evaluateAll(b => b.every(e => e.getBoundingClientRect().height >= 44))));
  await page.keyboard.press('Escape');
  check(P, 'Escape closes and returns focus to Upload', (await up.getAttribute('aria-expanded')) === 'false' && (await activeLabel(page)) === 'Upload');
  await up.click();
  await page.keyboard.press('Tab'); await page.keyboard.press('Tab'); await page.keyboard.press('Tab'); await page.keyboard.press('Tab');
  await page.waitForTimeout(150);
  check(P, 'tabbing past the last option closes the panel (no trap)', (await up.getAttribute('aria-expanded')) === 'false');
  await up.click();
  await page.mouse.click(1000, 700);
  check(P, 'outside press closes the options', (await up.getAttribute('aria-expanded')) === 'false');
  await up.click();
  const [files] = await Promise.all([page.waitForEvent('filechooser'), page.getByRole('button', { name: 'Add files' }).click()]);
  await files.setFiles({ name: `picked-${runId}.txt`, mimeType: 'text/plain', buffer: Buffer.from('Picked file body.') });
  await page.waitForSelector(`.queue-item:has-text("picked-${runId}.txt") .phase-saved`, { timeout: 60000 });
  check(P, 'Add files -> chooser -> saved through the queue; focus back on Upload', (await activeLabel(page)) === 'Upload');
  await up.click();
  const [folder] = await Promise.all([page.waitForEvent('filechooser'), page.getByRole('button', { name: 'Add folder' }).click()]);
  check(P, 'Add folder opens the directory chooser', await folder.element().evaluate(e => e.hasAttribute('webkitdirectory')));
  await folder.setFiles([]).catch(() => {});
  await addFile(page, `second-${runId}.txt`, 'Second article body.');

  const menu = page.locator('button[aria-label="Saved articles"]');
  const mBox = await menu.boundingBox();
  check(P, 'menu button >= 44px with a two-line glyph', mBox.width >= 44 && mBox.height >= 44 && (await menu.evaluate(b => b.querySelector('path').getAttribute('d').split('M').length - 1)) === 2, mBox);
  check(P, 'saved list is not in the primary workspace', (await page.locator('.intake .document-list').count()) === 0 && !(await shown(page, '.saved-panel')));
  await menu.hover();
  await page.waitForTimeout(100);
  const mTip = await page.locator('.header-start .tip').boundingBox();
  check(P, 'menu tooltip "Saved articles" sits beside the button, clear of the composer', (await page.locator('.header-start .tip').innerText()) === 'Saved articles' && !overlaps(mTip, mBox) && mTip.y + mTip.height <= (await page.locator('.composer').boundingBox()).y + 1, { mTip, mBox });
  await menu.click();
  const dialog = page.locator('[role=dialog]');
  check(P, 'menu opens a modal dialog; trigger expanded; focus on Close', (await dialog.isVisible()) && (await dialog.getAttribute('aria-modal')) === 'true' && (await menu.getAttribute('aria-expanded')) === 'true' && (await activeLabel(page)) === 'Close saved articles');
  check(P, 'page behind is inert', await page.evaluate(() => ['.app-header', '.workspace-grid', '.skip-link'].every(s => document.querySelector(s).inert)));
  const rows = page.locator('.saved-panel .document-item');
  check(P, 'rows are list items with a name and a saved date, >= 44px tall, current one marked', (await page.locator('.saved-panel ul.document-list > li').count()) >= 2 && /\d{4}|\d{1,2}:\d{2}/.test(await rows.first().innerText()) && (await rows.evaluateAll(b => b.every(e => e.getBoundingClientRect().height >= 44))) && (await page.locator('.saved-panel [aria-current=true]').count()) === 1);
  for (let i = 0; i < 6 && !(await page.evaluate(() => document.activeElement.classList.contains('document-item'))); i++) await page.keyboard.press('Tab');
  const ring = await page.evaluate(() => { const e = document.activeElement, s = getComputedStyle(e); return { row: e.classList.contains('document-item'), style: s.outlineStyle, offset: s.outlineOffset }; });
  check(P, 'keyboard focus ring on a row is drawn inside the scroll container (not clipped)', ring.row && ring.style !== 'none' && parseFloat(ring.offset) < 0, ring);
  const stops = [];
  for (let i = 0; i < 8; i++) { await page.keyboard.press('Tab'); stops.push(await page.evaluate(() => { const a = document.activeElement; return a === document.body ? 'body' : a.closest('[role=dialog]') ? 'dialog' : 'OUTSIDE'; })); }
  check(P, 'Tab never lands on inert page content', stops.every(s => s !== 'OUTSIDE'), stops);
  await page.keyboard.press('Escape');
  check(P, 'Escape closes; focus returns to the menu button; page interactive again', !(await dialog.isVisible()) && (await activeLabel(page)) === 'Saved articles' && (await page.evaluate(() => !document.querySelector('.workspace-grid').inert)));
  await menu.click(); await page.locator('button[aria-label="Close saved articles"]').click();
  check(P, 'Close button closes and restores focus', !(await dialog.isVisible()) && (await activeLabel(page)) === 'Saved articles');
  await menu.click(); await page.mouse.click(1300, 450);
  check(P, 'scrim click closes', !(await dialog.isVisible()));

  const before = await page.locator('.queue-item').count();
  await menu.click();
  const pasted = await page.evaluate(() => { const dt = new DataTransfer(); dt.setData('text/plain', 'must not import while the dialog is open'); document.activeElement.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true })); return true; });
  await page.waitForTimeout(300);
  check(P, 'paste while the dialog is open imports nothing behind it', pasted && (await page.locator('.queue-item').count()) === before);
  await page.route('**/api/documents?q=zz*', route => route.fulfill({ status: 500, contentType: 'application/json', body: '{"error":"Search backend unavailable"}' }));
  await page.fill('#search', 'zz');
  await page.keyboard.press('Enter');
  await page.waitForSelector('.saved-panel .notice.error', { timeout: 5000 }).catch(() => {});
  check(P, 'a failed search is shown inside the dialog', (await shown(page, '.saved-panel .notice.error')) && /unavailable/.test(await page.locator('.saved-panel .notice.error').innerText()));
  await page.unroute('**/api/documents?q=zz*');
  await page.locator('.saved-panel .notice.error button').click();
  await page.keyboard.press('Escape');

  // Selecting: closes, opens, focuses the heading, records the URL, and a double-click cannot reach the page underneath.
  await menu.click();
  await page.evaluate(() => { window.__stray = 0; document.addEventListener('click', event => { if (!event.target.closest('.saved-layer')) window.__stray++; }); });
  const current = (await page.locator('.reader-heading h2').innerText()).trim();
  const target = page.locator('.saved-panel .document-item', { hasText: `second-${runId}.txt` }).first();
  check(P, 'the article being chosen is not the one already open', !current.includes(`second-${runId}`), current);
  await target.dblclick();
  await page.waitForFunction(() => document.activeElement === document.querySelector('.reader-heading h2'), null, { timeout: 20000 }).catch(() => {});
  const chosen = { closed: !(await dialog.isVisible()), heading: await page.locator('.reader-heading h2').innerText(), focusOnHeading: await page.evaluate(() => document.activeElement === document.querySelector('.reader-heading h2')), active: await page.evaluate(() => document.activeElement?.outerHTML.slice(0, 80)), url: page.url() };
  check(P, 'choosing an article closes the dialog, opens it, and lands focus on its heading', chosen.closed && chosen.heading.includes(`second-${runId}`) && chosen.focusOnHeading && new URL(chosen.url).searchParams.has('document'), chosen);
  check(P, 'the second click of a double-click does not hit the page underneath', (await page.evaluate(() => window.__stray)) === 0);
  await page.goBack();
  await page.waitForTimeout(400);
  check(P, 'browser Back still returns to the previous article', (await page.locator('.reader-heading h2').innerText()).includes(`picked-${runId}`), await page.locator('.reader-heading h2').innerText());
  await menu.click();
  await page.goForward();
  await page.waitForTimeout(400);
  check(P, 'history navigation with the dialog open closes it and returns focus to the menu button', !(await dialog.isVisible()) && (await activeLabel(page)) === 'Saved articles');
  check(P, 'import queue still present', (await page.locator('.queue-item').count()) >= 2);
  check(P, 'no page errors', errors.length === 0, errors);
  await context.close();
}

async function slowSelection(browser) {
  const P = 'focus-not-stolen';
  const { context, page, errors } = await boot(browser, { viewport: { width: 1200, height: 800 } });
  await addFile(page, `slow-a-${runId}.txt`, 'First slow article.');
  await addFile(page, `slow-b-${runId}.txt`, 'Second slow article.');
  await page.locator('.queue-item', { hasText: `slow-a-${runId}` }).locator('.queue-open').click();
  let release;
  const gate = new Promise(resolveGate => { release = resolveGate; });
  await page.route(/\/api\/documents\/[^/?]+$/, async route => { await gate; await route.continue(); });
  await page.locator('button[aria-label="Saved articles"]').click();
  await page.locator('.saved-panel .document-item', { hasText: `slow-b-${runId}` }).first().click();
  await page.locator('textarea#source-paste').click();
  await page.keyboard.type('hello wor');
  release();
  await page.waitForFunction(name => document.querySelector('.reader-heading h2')?.textContent.includes(name), `slow-b-${runId}`, { timeout: 20000 });
  await page.keyboard.type('ld');
  check(P, 'a slow load does not pull focus away from what the user is typing', (await page.locator('textarea#source-paste').inputValue()) === 'hello world');
  check(P, 'no page errors', errors.length === 0, errors);
  await context.close();
}

async function touch(browser) {
  const P = 'iphone-14-390x664-touch';
  const { context, page, errors } = await boot(browser, { ...devices['iPhone 14'] });
  const up = page.locator('button[aria-label="Upload"]');
  await up.tap();
  check(P, 'one tap opens the labelled options (no tooltip-only first tap, no sticky tooltip)', (await up.getAttribute('aria-expanded')) === 'true' && (await page.locator('.menu-title').innerText()) === 'Upload' && !(await shown(page, '.composer .tip')));
  const ob = await page.locator('.add-menu-options').boundingBox(), vp = page.viewportSize();
  check(P, 'options panel fully on screen', ob.x >= 0 && ob.x + ob.width <= vp.width && ob.y + ob.height <= vp.height, ob);
  await page.touchscreen.tap(300, 560);
  check(P, 'tap outside closes the options', (await up.getAttribute('aria-expanded')) === 'false');
  check(P, 'drop hint is hidden where nothing can be dropped', !(await shown(page, '.composer-hint')));
  await up.tap();
  const [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.getByRole('button', { name: 'Add files' }).tap()]);
  await chooser.setFiles({ name: `phone-${runId}.txt`, mimeType: 'text/plain', buffer: Buffer.from('From a phone.') });
  await page.waitForSelector(`.queue-item:has-text("phone-${runId}.txt") .phase-saved`, { timeout: 60000 });
  check(P, 'tap, tap, system picker, imported', true);
  await page.locator('button[aria-label="Saved articles"]').tap();
  const pb = await page.locator('.saved-panel').boundingBox();
  check(P, 'dialog leaves a scrim strip and does not overflow sideways', pb.width <= 390 * 0.88 + 1 && (await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)), pb);
  await page.touchscreen.tap(375, 300);
  check(P, 'tap on the scrim closes it', !(await shown(page, '.saved-panel')));
  check(P, 'no page errors', errors.length === 0, errors);
  await context.close();
}

async function smallPhone(browser) {
  const P = 'iphone-se-320x568';
  const { context, page, errors } = await boot(browser, { ...devices['iPhone SE'] });
  for (const n of ['a', 'b', 'c']) await addFile(page, `se-${n}-${runId}.txt`, `Article ${n}.`);
  await page.locator('button[aria-label="Saved articles"]').tap();
  await page.locator('.saved-panel .document-item', { hasText: `se-a-${runId}` }).first().tap();
  await page.waitForFunction(name => document.querySelector('.reader-heading h2')?.textContent.includes(name), `se-a-${runId}`, { timeout: 20000 });
  await page.waitForTimeout(400);
  const top = await page.evaluate(() => document.querySelector('.reader-heading h2').getBoundingClientRect().top);
  check(P, 'after choosing an article the reader start is on screen (not left below the fold)', top < 568 - 150, top);
  await page.evaluate(() => { document.documentElement.style.fontSize = '36px'; });
  await page.waitForTimeout(200);
  check(P, '200% text: no sideways scroll; brand stays on one line', (await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)) && (await page.evaluate(() => document.querySelector('.brand h1').getBoundingClientRect().height < 90)));
  await page.locator('button[aria-label="Upload"]').tap();
  await page.waitForTimeout(150);
  const ob = await page.locator('.add-menu-options').boundingBox();
  check(P, '200% text: options panel is fully in view', ob.x >= 0 && ob.x + ob.width <= 321 && ob.y >= 0 && ob.y + ob.height <= 569, ob);
  await page.touchscreen.tap(300, 40);
  await page.locator('button[aria-label="Saved articles"]').tap();
  const heading = await page.locator('.saved-head h2').evaluate(h => ({ h: h.getBoundingClientRect().height, lh: parseFloat(getComputedStyle(h).lineHeight) }));
  const panel = await page.locator('.saved-panel').evaluate(p => ({ scrolls: p.scrollHeight >= p.clientHeight, within: p.getBoundingClientRect().right <= innerWidth }));
  check(P, '200% text: dialog title wraps by word and the panel stays in view', heading.h / heading.lh <= 3 && panel.within && panel.scrolls !== undefined, { heading, panel });
  check(P, 'no page errors', errors.length === 0, errors);
  await context.close();
}

async function appearance(browser) {
  const P = 'appearance';
  const forced = await boot(browser, { viewport: { width: 1000, height: 700 }, forcedColors: 'active' });
  await forced.page.locator('button[aria-label="Upload"]').hover();
  await forced.page.waitForTimeout(100);
  check(P, 'forced colors: the tooltip has a hairline outline, not a thick bar', await forced.page.locator('.composer .tip').evaluate(t => parseFloat(getComputedStyle(t).borderBottomWidth) <= 1));
  await forced.context.close();
  const dark = await boot(browser, { viewport: { width: 1000, height: 700 }, colorScheme: 'dark' });
  await dark.page.locator('button[aria-label="Upload"]').hover();
  await dark.page.waitForTimeout(100);
  const ratio = await dark.page.locator('.composer .tip').evaluate(t => { const s = getComputedStyle(t), lum = c => { const m = c.match(/\d+/g).map(Number).map(v => { v /= 255; return v <= .03928 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4; }); return .2126 * m[0] + .7152 * m[1] + .0722 * m[2]; }, [a, b] = [lum(s.backgroundColor), lum(s.color)].sort((x, y) => y - x); return (a + .05) / (b + .05); });
  check(P, 'dark mode: tooltip contrast >= 4.5:1', ratio >= 4.5, ratio);
  await dark.context.close();
}

await ensureSchema();
const browser = await chromium.launch();
for (const [name, run] of [['wide 1440', b => wideDesktop(b, 1440, 900)], ['wide 1120', b => wideDesktop(b, 1120, 800)], ['desktop', desktop], ['slow selection', slowSelection], ['touch', touch], ['small phone', smallPhone], ['appearance', appearance]]) {
  try { await run(browser); } catch (error) { check(name, 'scenario completed', false, String(error).slice(0, 500)); }
}
await browser.close();
const failed = results.filter(r => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
