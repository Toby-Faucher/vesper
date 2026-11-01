// Screenshot scenes for review: node e2e/shots.mjs [names...] (npm run shots -- login room). All scenes when none are named.
// Starts the release server like the e2e tests (needs ../target/release/vesper-server and dist/), signs up the owner alice
// and the member bob, seeds three rooms and 40 messages, then writes e2e/shots/<name>.png at 1440x900, full page.
// The `tokens` and `gallery` scenes shoot /_gallery from a Vite dev server on a spare port, since the build leaves it out.
import { spawn } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import { ORIGIN, browser, cli, signup, sleep, startServer } from './lib.mjs';

const VIEWPORT = { width: 1440, height: 900 };
const out = fileURLToPath(new URL('shots/', import.meta.url));
const speech = fileURLToPath(new URL('fixtures/speech.wav', import.meta.url));

const scenes = new Map();
// fn({ alice, bob, api, open, shot }) returns the page to shoot. alice and bob are signed-in { page, errors }; api(page, method,
// path, body) calls the API from that page (cookie and Origin as the app sends them); open(url) is a fresh signed-out page.
// A scene that takes more than one picture calls shot(page, name, { fullPage }) itself and returns nothing.
const scene = (name, fn) => scenes.set(name, fn);

async function api(page, method, path, body) {
  for (;;) {
    const r = await page.evaluate(
      async ([method, path, body]) => {
        const r = await fetch(path, {
          method,
          headers: body === undefined ? undefined : { 'content-type': 'application/json' },
          body: body === undefined ? undefined : JSON.stringify(body),
        });
        return { status: r.status, json: r.status === 204 ? null : await r.json().catch(() => null) };
      },
      [method, path, body],
    );
    if (r.status === 429) { await sleep(500); continue; } // the message limit: 10, then 1 a second
    if (r.status >= 400) throw new Error(`${method} ${path}: ${r.status}`);
    return r.json;
  }
}

async function home(page) {
  await page.goto(`${ORIGIN}/`);
  await page.getByText('Signed in as').waitFor();
}
async function joinVoice(page) {
  await page.getByRole('button', { name: 'Join' }).click();
  await page.getByRole('button', { name: 'Leave' }).waitFor();
}

// The Vite dev server for the gallery pages, started once on a spare port and stopped at the end.
let dev;
function devServer() {
  dev ??= (async () => {
    const port = await new Promise((r) => { const s = createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)); }); });
    const web = fileURLToPath(new URL('..', import.meta.url));
    const vite = spawn('node_modules/.bin/vite', ['--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: web, stdio: 'ignore' });
    stops.push(() => vite.kill('SIGTERM'));
    const url = `http://127.0.0.1:${port}`;
    for (let i = 0; i < 80 && !(await fetch(url).then((r) => r.ok, () => false)); i++) await sleep(250);
    return url;
  })();
  return dev;
}

scene('tokens', async ({ open }) => {
  const page = await open(`${await devServer()}/_gallery`);
  await page.getByRole('heading', { name: 'Tokens' }).waitFor();
  return page;
});

// The primitives. Checks the keyboard paths first (focus ring on every Tab stop, dialog and menu focus, Esc, arrows), then
// forces :hover and :focus-visible on the [data-force] examples and takes `gallery`, then opens a dialog with a menu inside
// it and takes `gallery-open`, then a dialog on top of it and takes `gallery-nested`.
scene('gallery', async ({ open, shot }) => {
  const page = await open(`${await devServer()}/_gallery?primitives`);
  await page.getByRole('heading', { name: 'Primitives' }).waitFor();
  const must = (ok, what) => { if (!ok) throw new Error(`gallery: ${what}`); };
  const focused = () => page.evaluate(() => { const a = document.activeElement; return a?.getAttribute('aria-label') || a?.textContent?.trim() || a?.tagName; });
  const expectFocus = async (name, what) => { const f = await focused(); must(f === name, `${what}: focus on ${JSON.stringify(f)}, want ${JSON.stringify(name)}`); };

  // Every Tab stop shows an outline. Stops once focus comes back to the first element.
  const tabStops = [];
  for (let i = 0; i < 300; i++) {
    await page.keyboard.press('Tab');
    const s = await page.evaluate(() => {
      const a = document.activeElement;
      if (!a || a === document.body) return null;
      const cs = getComputedStyle(a);
      return { id: a.outerHTML.slice(0, 80), name: a.getAttribute('aria-label') || a.textContent?.trim().slice(0, 30) || a.tagName, outline: cs.outlineStyle !== 'none' && cs.outlineWidth !== '0px' };
    });
    if (!s) continue;
    if (tabStops.length && s.id === tabStops[0].id) break;
    tabStops.push(s);
  }
  const bare = tabStops.filter((s) => !s.outline).map((s) => s.name);
  must(tabStops.length > 30, `only ${tabStops.length} Tab stops`);
  must(!bare.length, `no focus outline on ${bare.join(', ')}`);
  console.log(`gallery: ${tabStops.length} Tab stops, each with a focus ring`);

  // Dialog: focus moves in, Esc closes, focus returns; a backdrop click closes too.
  const opener = page.getByRole('button', { name: 'Create invite' });
  const dialog = page.getByRole('dialog', { name: 'Create an invite' });
  await opener.click();
  await dialog.waitFor();
  must(await dialog.evaluate((d) => d.contains(document.activeElement)), 'dialog did not take focus');
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
  await expectFocus('Create invite', 'after Esc on the dialog');
  await opener.click();
  await dialog.waitFor();
  await page.mouse.click(8, 8);
  await dialog.waitFor({ state: 'hidden' });
  await expectFocus('Create invite', 'after a backdrop click');

  // Menu: roving focus, typeahead, Esc, Enter, Tab, and the trigger toggles it.
  const trigger = page.getByRole('button', { name: 'Message actions' });
  const menu = page.getByRole('menu', { name: 'Message actions' });
  must((await trigger.getAttribute('aria-haspopup')) === 'menu', 'menu trigger has no aria-haspopup');
  await trigger.click();
  await menu.waitFor();
  must((await trigger.getAttribute('aria-expanded')) === 'true', 'aria-expanded not true while open');
  await expectFocus('Reply', 'menu open');
  for (const [key, want] of [['ArrowDown', 'Edit'], ['End', 'Delete'], ['ArrowDown', 'Reply'], ['ArrowUp', 'Delete'], ['ArrowUp', 'Copy link'], ['Home', 'Reply'], ['p', 'Pin'], ['c', 'Copy link']]) {
    await page.keyboard.press(key);
    await expectFocus(want, `menu after ${key}`);
  }
  await page.keyboard.press('Escape');
  await menu.waitFor({ state: 'hidden' });
  await expectFocus('Message actions', 'after Esc on the menu');
  // The toggle event that reports the close is async: wait for the attribute.
  await page.waitForFunction(() => document.querySelector('[aria-label="Message actions"]')?.getAttribute('aria-expanded') === 'false', null, { timeout: 2000 })
    .catch(() => must(false, 'aria-expanded not false after close'));
  await page.keyboard.press('Enter');
  await menu.waitFor();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await menu.waitFor({ state: 'hidden' });
  await page.getByText('Last action: Edit').waitFor();
  await expectFocus('Message actions', 'after choosing an item');
  await page.keyboard.press('Space');
  await menu.waitFor();
  await page.keyboard.press('Tab');
  await menu.waitFor({ state: 'hidden' });
  await expectFocus('Message actions', 'after Tab in the menu');
  await trigger.click();
  await menu.waitFor();
  await trigger.click();
  await menu.waitFor({ state: 'hidden' });

  // Popover: Esc closes and focus returns.
  const card = page.getByRole('button', { name: 'Show person card' });
  await card.click();
  await page.getByRole('dialog', { name: 'bob' }).waitFor();
  await page.keyboard.press('Escape');
  await page.getByRole('dialog', { name: 'bob' }).waitFor({ state: 'hidden' });
  await expectFocus('Show person card', 'after Esc on the popover');

  // aria: each trigger names its popup kind and points aria-controls at the layer it opens.
  for (const [name, popup, role] of [['Message actions', 'menu', 'menu'], ['Show person card', 'dialog', 'dialog'], ['Create invite', 'dialog', 'dialog']]) {
    const t = page.getByRole('button', { name, exact: true });
    must((await t.getAttribute('aria-haspopup')) === popup, `${name}: aria-haspopup is not ${popup}`);
    const id = await t.getAttribute('aria-controls');
    must(id && (await page.evaluate((id) => document.getElementById(id)?.getAttribute('role') ?? document.getElementById(id)?.tagName.toLowerCase(), id)) === role,
      `${name}: aria-controls ${JSON.stringify(id)} does not point at a ${role}`);
  }

  // Nested dialogs and the scroll lock: the page does not scroll behind a modal, Esc closes only the top dialog and focus
  // goes back to that dialog's opener; the lock goes with the last dialog.
  const overflow = () => page.evaluate(() => getComputedStyle(document.documentElement).overflow);
  const revoke = page.getByRole('dialog', { name: 'Revoke all invite links?' });
  must((await overflow()) !== 'hidden', 'page locked before any dialog');
  await opener.click();
  await dialog.waitFor();
  must((await overflow()) === 'hidden', 'no scroll lock with a dialog open');
  const y = await page.evaluate(() => scrollY);
  await page.mouse.wheel(0, 600);
  await sleep(200);
  must((await page.evaluate(() => scrollY)) === y, 'the page scrolled behind the dialog');
  await page.getByRole('button', { name: 'Revoke all links' }).click();
  await revoke.waitFor();
  must(await revoke.evaluate((d) => d.contains(document.activeElement)), 'the inner dialog did not take focus');
  await page.keyboard.press('Escape');
  await revoke.waitFor({ state: 'hidden' });
  must(await dialog.isVisible(), 'Esc on the inner dialog closed the outer one');
  await expectFocus('Revoke all links', 'after Esc on the inner dialog');
  must((await overflow()) === 'hidden', 'scroll lock released while the outer dialog is open');
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
  await expectFocus('Create invite', 'after Esc on the outer dialog');
  must((await overflow()) !== 'hidden', 'scroll lock kept after the last dialog closed');

  // A browser without showPopover({ source }): light dismiss on the anchor's pointerdown must not reopen on its click.
  const old = await open(`${await devServer()}/_gallery?primitives`);
  await old.addInitScript(() => {
    const show = HTMLElement.prototype.showPopover;
    HTMLElement.prototype.showPopover = function () { return show.call(this); };
  });
  await old.reload();
  const oldTrigger = old.getByRole('button', { name: 'Message actions' });
  const oldMenu = old.getByRole('menu', { name: 'Message actions' });
  await oldTrigger.click();
  await oldMenu.waitFor();
  await oldTrigger.click();
  await oldMenu.waitFor({ state: 'hidden' });
  await sleep(300);
  must(!(await oldMenu.isVisible()), 'without source: a second click on the trigger reopened the menu');
  await oldTrigger.click();
  await oldMenu.waitFor().catch(() => must(false, 'without source: the third click did not open the menu'));
  await old.keyboard.press('Escape');
  await oldMenu.waitFor({ state: 'hidden' });
  await old.close();

  // Closed layers, with hover and focus forced on the [data-force] examples.
  await page.evaluate(() => { document.activeElement?.blur(); scrollTo(0, 0); });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('DOM.enable');
  await cdp.send('CSS.enable');
  const { root } = await cdp.send('DOM.getDocument', { depth: -1 });
  for (const kind of ['hover', 'focus-visible']) {
    const { nodeIds } = await cdp.send('DOM.querySelectorAll', { nodeId: root.nodeId, selector: `[data-force="${kind}"]` });
    for (const nodeId of nodeIds) await cdp.send('CSS.forcePseudoState', { nodeId, forcedPseudoClasses: [kind] });
  }
  await sleep(700); // the tooltip shows after 400 ms
  await shot(page, 'gallery');

  // Open layers: the dialog with its presets menu open inside it.
  await opener.click();
  await dialog.waitFor();
  await page.getByRole('button', { name: 'Presets' }).click();
  await page.getByRole('menu', { name: 'Presets' }).waitFor();
  await sleep(300); // the enter motion
  await shot(page, 'gallery-open', { fullPage: false }); // a full-page capture stretches the page under the fixed backdrop
  await page.keyboard.press('End');
  await page.keyboard.press('Enter');
  await page.getByRole('menu', { name: 'Presets' }).waitFor({ state: 'hidden' });
  must(await dialog.isVisible(), 'choosing in the inner menu closed the dialog');
  await expectFocus('Presets', 'after the inner menu');
  await page.getByRole('button', { name: 'Revoke all links' }).click();
  await revoke.waitFor();
  await sleep(300);
  await shot(page, 'gallery-nested', { fullPage: false });
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
});

scene('login', async ({ open }) => {
  const page = await open(`${ORIGIN}/login`);
  await page.getByRole('button', { name: 'Sign in' }).waitFor();
  return page;
});

scene('signup', async ({ alice, open }) => {
  const { url } = await api(alice.page, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 });
  const page = await open(url);
  await page.getByRole('button', { name: 'Create account' }).waitFor();
  return page;
});

scene('room', async ({ alice, bob }) => {
  await home(alice.page);
  await home(bob.page);
  await joinVoice(alice.page);
  await joinVoice(bob.page);
  await alice.page.locator('li', { hasText: 'bob' }).waitFor();
  await sleep(1000);
  return alice.page;
});

scene('settings', async ({ alice }) => {
  await home(alice.page);
  await alice.page.getByRole('button', { name: 'Audio settings' }).click();
  await alice.page.getByLabel('Audio settings').waitFor();
  return alice.page;
});

scene('diagnostics', async ({ alice }) => {
  await home(alice.page);
  await joinVoice(alice.page);
  await alice.page.getByRole('button', { name: 'Diagnostics' }).click();
  await sleep(2500); // the stats refresh
  return alice.page;
});

// Main-thread CPU on bob's page in a two-person call: CDP TaskDuration over 10 s, printed, no picture. Run it three times
// and take the median (docs/dev.md has the baseline).
scene('cpu-baseline', async ({ alice, bob }) => {
  await home(alice.page);
  await home(bob.page);
  await joinVoice(alice.page);
  await joinVoice(bob.page);
  await bob.page.locator('li', { hasText: 'alice' }).waitFor();
  await sleep(2000); // let the call settle
  const cdp = await bob.page.context().newCDPSession(bob.page);
  await cdp.send('Performance.enable');
  const task = async () => (await cdp.send('Performance.getMetrics')).metrics.find((m) => m.name === 'TaskDuration').value;
  const t0 = await task();
  await sleep(10000);
  console.log(`cpu-baseline: ${Math.round((await task() - t0) * 1000)} ms per 10 s`);
});

const LINES = [
  'morning all', 'anyone up for a game tonight?', 'I can do 9', 'same, after dinner', 'what are we playing',
  'the co-op one again?', 'sure', 'I fixed my mic, tell me if it still crackles', 'sounds fine to me', 'link to the patch notes: https://example.com/notes',
];

const names = process.argv.slice(2);
for (const n of names) if (!scenes.has(n)) throw new Error(`unknown scene ${n}; known: ${[...scenes.keys()].join(' ')}`);
const run = names.length ? names : [...scenes.keys()];

const stops = [];
const stop = await startServer();
let failed = false;
try {
  await mkdir(out, { recursive: true });
  const fresh = [];
  const open = async (url) => {
    const b = await browser(`fresh${fresh.length}`, [], { viewport: VIEWPORT });
    fresh.push(b);
    await b.page.goto(url);
    return b.page;
  };
  const alice = await browser('alice', [`--use-file-for-fake-audio-capture=${speech}`], { viewport: VIEWPORT });
  await signup(alice.page, await cli(['create-owner']), 'alice');
  const bobLink = (await api(alice.page, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 })).url;
  const bob = await browser('bob', [], { viewport: VIEWPORT });
  await signup(bob.page, bobLink, 'bob');
  for (const name of ['music', 'lobby']) await api(alice.page, 'POST', '/api/rooms', { name });
  const general = (await api(alice.page, 'GET', '/api/rooms')).find((r) => r.name === 'general');
  for (let i = 0; i < 40; i++) {
    const from = i % 3 === 2 ? bob : alice;
    await api(from.page, 'POST', `/api/rooms/${general.id}/messages`, { body: LINES[i % LINES.length] });
  }

  const shot = async (page, name, { fullPage = true } = {}) => {
    const path = `${out}${name}.png`;
    await page.screenshot({ path, fullPage });
    console.log(path);
  };
  for (const name of run) {
    const page = await scenes.get(name)({ alice, bob, api, open, shot });
    if (page) await shot(page, name);
  }
  const errors = [alice, bob, ...fresh].flatMap((b) => b.errors);
  if (errors.length) {
    console.error('page errors:\n' + errors.join('\n'));
    failed = true;
  }
} finally {
  for (const s of stops) s();
  await stop();
}
process.exit(failed ? 1 : 0);
