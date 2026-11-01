// Screenshot scenes for review: node e2e/shots.mjs [names...] (npm run shots -- login room). All scenes when none are named.
// Starts the release server like the e2e tests (needs ../target/release/vesper-server and dist/), signs up the owner alice
// and the member bob, then writes e2e/shots/<name>.png at 1440x900, full page. Scenes that need content call seed() (two more
// rooms and 40 messages in general), so shell-empty runs before it; reconnecting restarts the server, so it runs last.
// The `tokens` and `gallery` scenes shoot /_gallery from a Vite dev server on a spare port, since the build leaves it out.
import { spawn } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
import { createServer } from 'node:net';
import { crc32, deflateSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { ORIGIN, browser, cli, inVoice, openRoom, signedIn, signup, sleep, startServer } from './lib.mjs';

const VIEWPORT = { width: 1440, height: 900 };
const out = fileURLToPath(new URL('shots/', import.meta.url));
const speech = fileURLToPath(new URL('fixtures/speech.wav', import.meta.url));

const scenes = new Map();
// fn({ alice, bob, api, open, shot, seed, server }) returns the page to shoot. alice and bob are signed-in { page, errors }; api(page, method,
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
  await signedIn(page);
  await page.waitForURL(/\/r\/\d+$/);
}
async function joinVoice(page) {
  await page.getByRole('button', { name: 'Join' }).click();
  await page.getByRole('button', { name: 'Leave' }).waitFor();
}
// Shows the page with nothing focused and the pointer out of the way, so no hover or focus ring is in the picture.
async function rest(page) {
  await page.evaluate(() => document.activeElement?.blur());
  await page.mouse.move(1439, 899);
  await sleep(300);
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
  // Esc, then open again at once: the close event of the first close arrives late and must not close the new one, and
  // onclose ran once per close (Esc and the backdrop click: 2).
  const closes = () => page.getByRole('status').filter({ hasText: 'Last action' }).getAttribute('data-closes').then(Number);
  const before = await closes();
  await opener.click();
  await dialog.waitFor();
  await page.keyboard.press('Escape');
  await opener.click();
  await dialog.evaluate((d) => d.dispatchEvent(new Event('close'))); // a late close event from the Esc
  await sleep(100);
  must(await dialog.isVisible(), 'a late close event closed the reopened dialog');
  must((await closes()) === before + 1, `onclose ran ${(await closes()) - before} times for one Esc`);
  must(before === 2, `onclose ran ${before} times for Esc and a backdrop click`);
  // Removed while open (a re-key, a sign out): focus still goes back to the opener.
  await page.evaluate(() => window.__unmountDialog());
  await dialog.waitFor({ state: 'detached' });
  await expectFocus('Create invite', 'after the open dialog was removed');
  await page.reload(); // the dialog is gone from this page now
  await page.getByRole('button', { name: 'Create invite' }).waitFor();

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

// Sign in: empty, a wrong password, the rate limit (faked), and the narrow window.
scene('login', async ({ open, shot }) => {
  const page = await open(`${ORIGIN}/login`);
  await page.getByRole('button', { name: 'Sign in' }).waitFor();
  await rest(page);
  await shot(page, 'login');
  await page.getByLabel('Username').fill('alice');
  await page.getByLabel('Password', { exact: true }).fill('test-password-9');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await page.getByText('Wrong username or password.').waitFor();
  await shot(page, 'login-error'); // focus stays on the error, as a person sees it
  await page.route('**/api/login', (r) => r.fulfill({ status: 429, contentType: 'application/json', body: '{"code":"rate_limited"}' }));
  await page.getByLabel('Password', { exact: true }).fill('test-password-9');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await page.getByText('Too many tries. Wait a minute and try again.').waitFor();
  await shot(page, 'login-429');
  await page.setViewportSize({ width: 1024, height: 640 });
  await rest(page);
  await shot(page, 'login-narrow');
});

scene('signup', async ({ alice, open, shot }) => {
  const { url } = await api(alice.page, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 });
  const page = await open(url);
  await page.getByRole('button', { name: 'Create account' }).waitFor();
  await rest(page);
  await shot(page, 'signup');
  await page.goto(`${ORIGIN}/invite#not-a-code`);
  await page.getByText('This invite link does not work.', { exact: false }).waitFor();
  await rest(page);
  await shot(page, 'signup-bad-link');
});

// The welcome steps of a new account whose fake mic plays speech: step 2 runs the mic test so the meter moves.
scene('welcome', async ({ alice, shot }) => {
  const { url } = await api(alice.page, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 });
  const { page } = await browser('welcome', [`--use-file-for-fake-audio-capture=${speech}`], { viewport: VIEWPORT });
  await page.addInitScript(() => Object.defineProperty(Notification, 'permission', { get: () => 'default' })); // headless says 'denied'
  await page.goto(url);
  await page.getByLabel('Username').fill('wren');
  await page.getByLabel('Display name').fill('Wren Okafor');
  await page.getByLabel('Password', { exact: true }).fill('test-password-1');
  await page.getByRole('button', { name: 'Create account' }).click();
  await page.getByRole('heading', { name: 'How others see you' }).waitFor();
  await page.getByRole('radio', { name: 'Sky' }).check();
  await rest(page);
  await shot(page, 'welcome-1');
  await page.getByRole('button', { name: 'Next' }).click();
  await page.getByRole('button', { name: 'Mic test (hear yourself)' }).click();
  await page.waitForFunction(() => Number(document.querySelector('[role=meter]')?.getAttribute('aria-valuenow')) > -40);
  await rest(page);
  await shot(page, 'welcome-2');
  await page.getByRole('button', { name: 'Next' }).click();
  await rest(page);
  await shot(page, 'welcome-3');
  await page.setViewportSize({ width: 1024, height: 640 });
  await page.getByRole('button', { name: 'Back' }).click();
  await rest(page);
  await shot(page, 'welcome-narrow');
  await page.context().browser().close();
});

// One room, nobody in voice, no messages: what a new owner sees first.
scene('shell-empty', async ({ alice }) => {
  await home(alice.page);
  await rest(alice.page);
  return alice.page;
});

// Five people in voice in music (two speaking, one muted, one deafened, one with a long name), unread and mention pills
// on the other rooms, and a long room name. Alice is in the call and reads general.
let house;
let extra = []; // carol, dave and Bartholomew, in the music call with alice and bob
const LONG_ROOM = 'weekend plans and the games list'; // 32 characters, the server's limit
function fullHouse({ alice, bob, api, seed }) {
  house ??= (async () => {
    await seed();
    const rooms = await api(alice.page, 'GET', '/api/rooms');
    const id = (name) => rooms.find((r) => r.name === name).id;
    for (const [name, display] of [['carol', 'carol'], ['dave', 'dave'], ['bart', 'Bartholomew Featherstonehaugh']]) {
      const { url } = await api(alice.page, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 });
      const b = await browser(name, [], { viewport: VIEWPORT });
      await signup(b.page, url, name, display);
      extra.push(b);
    }
    await home(alice.page);
    for (const p of [alice.page, bob.page, ...extra.map((b) => b.page)]) {
      if (p !== alice.page) await home(p);
      await openRoom(p, 'music');
      await joinVoice(p);
    }
    await extra[0].page.getByRole('button', { name: 'Mute', exact: true }).click();
    await extra[1].page.getByRole('button', { name: 'Deafen', exact: true }).click();
    await openRoom(alice.page, 'general');
    for (const body of ['set list for friday is up', 'who has the good speaker cable']) await api(bob.page, 'POST', `/api/rooms/${id('music')}/messages`, { body });
    for (const body of ['saturday works for me', '@alice can you book the room?', 'I will bring snacks'])
      await api(bob.page, 'POST', `/api/rooms/${id(LONG_ROOM)}/messages`, { body });
    for (const n of ['carol', 'dave', 'Bartholomew']) await inVoice(alice.page, n).waitFor();
    await alice.page.locator('li.speaking[data-speaking]').first().waitFor({ timeout: 10000 }).catch(() => console.warn('shell: nobody seen speaking'));
  })();
  return house;
}
scene('shell', async (ctx) => {
  await fullHouse(ctx);
  const page = ctx.alice.page;
  await rest(page);
  await ctx.shot(page, 'shell');
  // Folded into the avatar stack (automatic above five people, or with the chevron).
  await page.getByRole('button', { name: 'Hide people in voice in music' }).click();
  await rest(page);
  await ctx.shot(page, 'shell-folded');
  await page.getByRole('button', { name: 'Show people in voice in music' }).click();
});
scene('shell-collapsed', async (ctx) => {
  await fullHouse(ctx);
  const page = ctx.alice.page;
  await page.getByRole('button', { name: 'Hide sidebar' }).click();
  await rest(page);
  await ctx.shot(page, 'shell-collapsed');
  await page.getByRole('button', { name: 'Show sidebar' }).click();
});

// The call bar: alice reads general while in the music call with four others (two muted or deafened, one speaking).
const bar = (page) => page.getByRole('region', { name: 'Call' });
scene('call-bar', async (ctx) => {
  await fullHouse(ctx);
  const page = ctx.alice.page;
  await bar(page).locator('[data-speaking]').first().waitFor({ timeout: 10000 }).catch(() => console.warn('call-bar: nobody seen speaking'));
  await rest(page);
  await ctx.shot(page, 'call-bar');
  await page.getByRole('button', { name: 'Mute', exact: true }).click();
  await page.getByRole('button', { name: 'Deafen', exact: true }).click();
  await rest(page);
  await ctx.shot(page, 'call-muted-deafened');
  await page.getByRole('button', { name: 'Unmute' }).click();
  await page.getByRole('button', { name: 'Undeafen' }).click();
  // The narrowest main pane: 1024 px wide with the sidebar open.
  await page.setViewportSize({ width: 1024, height: 640 });
  await rest(page);
  await ctx.shot(page, 'call-narrow');
  await page.setViewportSize(VIEWPORT);
});
// Push to talk on (from Settings), first idle, then with the key held.
scene('call-ptt', async (ctx) => {
  await fullHouse(ctx);
  const page = ctx.alice.page;
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByLabel('Push to talk').check();
  await page.goBack();
  await page.getByText('Push to talk: hold Space').waitFor();
  await rest(page);
  await ctx.shot(page, 'call-ptt-idle');
  await page.keyboard.down('Space');
  await bar(page).getByText('Talking').waitFor();
  await sleep(300);
  await ctx.shot(page, 'call-ptt');
  await page.keyboard.up('Space');
  // The widest row at the narrowest main pane: the room block shrinks, nothing overflows.
  await page.setViewportSize({ width: 1024, height: 640 });
  await rest(page);
  await ctx.shot(page, 'call-ptt-narrow');
  const over = await bar(page).evaluate((e) => e.scrollWidth > e.clientWidth || e.getBoundingClientRect().right > innerWidth);
  if (over) throw new Error('call-ptt-narrow: the bar overflows');
  await page.setViewportSize(VIEWPORT);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByLabel('Push to talk').uncheck();
  await page.goBack();
});
// Bob is muted by the owner: Mute is disabled and the notice is in the status line.
scene('call-force-muted', async (ctx) => {
  await fullHouse(ctx);
  const me = await api(ctx.bob.page, 'GET', '/api/me');
  await api(ctx.alice.page, 'POST', `/api/users/${me.user.user_id}/force-mute`, { muted: true });
  await ctx.bob.page.getByText('A moderator muted you.').waitFor();
  await rest(ctx.bob.page);
  await ctx.shot(ctx.bob.page, 'call-force-muted');
  await api(ctx.alice.page, 'POST', `/api/users/${me.user.user_id}/force-mute`, { muted: false });
});
// A weak connection: alice's stats report 420 ms round trip (getStats is wrapped through the window.__pcs hook).
scene('call-weak', async (ctx) => {
  await fullHouse(ctx);
  const page = ctx.alice.page;
  await page.evaluate(() => {
    const pc = window.__pcs.at(-1);
    const get = pc.getStats.bind(pc);
    pc.getStats = async () => new Map([...(await get()).entries()].map(([k, v]) => [k, v.type === 'candidate-pair' ? { ...v, currentRoundTripTime: 0.42 } : v]));
  });
  await bar(page).getByText('Weak connection: 420 ms delay').waitFor({ timeout: 10000 });
  await rest(page);
  await ctx.shot(page, 'call-weak');
});
// Person cards. A member (bob) opens alice from the call bar and turns her up past 100 %; a moderator (dave) opens carol
// from the sidebar and sees the voice actions, then the Disconnect confirm. Dave on alice (the owner) gets no actions.
scene('person-card', async (ctx) => {
  await fullHouse(ctx);
  const page = ctx.bob.page;
  await bar(page).getByRole('button', { name: 'alice' }).click();
  const card = page.getByRole('dialog', { name: 'alice' });
  await card.getByRole('slider', { name: 'Volume for alice' }).fill('140');
  await card.getByText('Can distort').waitFor();
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'person-card');
  await card.getByRole('button', { name: 'Reset' }).click();
  await page.keyboard.press('Escape');
});
scene('person-card-mod', async (ctx) => {
  await fullHouse(ctx);
  const users = await api(ctx.alice.page, 'GET', '/api/users');
  const dave = users.find((u) => u.username === 'dave');
  await api(ctx.alice.page, 'PATCH', `/api/users/${dave.user_id}/role`, { role: 'moderator' });
  const page = extra[1].page; // dave's tab learns the new role from user_updated
  await inVoice(page, 'carol').getByRole('button').click();
  const card = page.getByRole('dialog', { name: 'carol' });
  await card.getByRole('button', { name: 'Remove from voice' }).waitFor();
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'person-card-mod');
  await card.getByRole('button', { name: 'Disconnect' }).click();
  await page.getByRole('dialog', { name: 'Disconnect carol?' }).waitFor();
  await sleep(300);
  await ctx.shot(page, 'person-card-confirm', { fullPage: false });
  await page.getByRole('button', { name: 'Cancel' }).click();
  await page.keyboard.press('Escape');
  await card.waitFor({ state: 'hidden' });
  await inVoice(page, 'alice').getByRole('button').click();
  const owner = page.getByRole('dialog', { name: 'alice' });
  await owner.getByText('Owner').waitFor();
  if (await owner.getByRole('button', { name: 'Disconnect' }).count()) throw new Error('person-card-mod: a moderator sees actions on the owner');
  await page.keyboard.press('Escape');
});

// A restricted room: alice (owner) sees it with its label; bob (member) opening its address gets the plain missing state.
// The Ctrl+K switcher on the full house: empty (unread rooms first, then people in voice, then actions), "gen" (rooms and
// actions mixed), scoped to actions with "> ", no match, and the shortcuts list opened from it.
async function switcher(ctx, query) {
  await fullHouse(ctx);
  const page = ctx.alice.page;
  await rest(page);
  await page.keyboard.press('Control+k');
  const box = page.getByRole('combobox', { name: 'Switch to' });
  await box.fill(query);
  await sleep(700); // past the results count delay
  return page;
}
const closeSwitcher = (page) => page.keyboard.press('Escape');
for (const [name, query] of [['switcher-empty', ''], ['switcher-query', 'gen'], ['switcher-actions', '> '], ['switcher-none', 'zzq']])
  scene(name, async (ctx) => {
    const page = await switcher(ctx, query);
    await ctx.shot(page, name);
    await closeSwitcher(page);
  });
// The switcher's dialog before its chunk has loaded (held back, then let through), and after a failed load.
for (const [name, fail] of [['switcher-loading', false], ['switcher-failed', true]])
  scene(name, async (ctx) => {
    await fullHouse(ctx);
    const page = ctx.alice.page;
    const chunk = '**/assets/switcher-chunk-*.js';
    let release;
    const held = new Promise((r) => (release = r));
    await page.route(chunk, async (r) => (fail ? r.abort() : (await held, r.continue())));
    await page.reload();
    await rest(page);
    await page.keyboard.press('Control+k');
    await page.keyboard.type('mus');
    await page.getByText(fail ? 'Could not load the switcher.' : 'Loading...').waitFor();
    await sleep(200);
    await ctx.shot(page, name);
    release();
    await page.unroute(chunk);
    await closeSwitcher(page);
  });
scene('shortcuts', async (ctx) => {
  const page = await switcher(ctx, '> keyboard');
  await page.keyboard.press('Enter');
  await page.getByRole('dialog', { name: 'Keyboard shortcuts' }).waitFor();
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'shortcuts');
  await page.keyboard.press('Escape');
});
scene('hidden-room', async ({ alice, bob, api, seed, shot }) => {
  await seed();
  const staff = await api(alice.page, 'POST', '/api/rooms', { name: 'staff' });
  await api(alice.page, 'PATCH', `/api/rooms/${staff.id}`, { restricted: true });
  await home(alice.page);
  await openRoom(alice.page, 'staff');
  await alice.page.getByText('Restricted').waitFor();
  await rest(alice.page);
  await shot(alice.page, 'room-restricted');
  await bob.page.goto(`${ORIGIN}/r/${staff.id}`);
  await bob.page.getByText('This room is not here.').waitFor();
  await rest(bob.page);
  await shot(bob.page, 'hidden-room');
});

scene('not-found', async ({ alice }) => {
  await alice.page.goto(`${ORIGIN}/no/such/page`);
  await alice.page.getByText('There is no page at this address.').waitFor();
  await rest(alice.page);
  return alice.page;
});

// The first Tab stop on a fresh load is the skip link, shown over the sidebar; Enter moves focus to the main pane.
scene('skip-link', async ({ alice, shot }) => {
  await home(alice.page);
  await alice.page.mouse.move(1439, 899);
  await alice.page.keyboard.press('Tab');
  const link = alice.page.getByRole('link', { name: 'Skip to main content' });
  if (!(await link.evaluate((el) => el === document.activeElement))) throw new Error('skip-link: not the first Tab stop');
  await sleep(300);
  await shot(alice.page, 'skip-link', { fullPage: false });
  await alice.page.keyboard.press('Enter');
  if (!(await alice.page.evaluate(() => document.activeElement?.id === 'main'))) throw new Error('skip-link: focus did not move to main');
});

// The timeline: a room "plans" with three days of talk (the harness moves the older messages back a day or two, since
// the server stamps them now), groups, a code block, a reply, an edited, a pinned and a deleted message, reactions, a
// long message, a mention of alice and a New row (alice read up to the deleted one). alice opens it at the New row.
let planned;
function plans({ alice, bob, api }) {
  planned ??= (async () => {
    const room = await api(alice.page, 'POST', '/api/rooms', { name: 'plans' });
    const by = { a: alice.page, b: bob.page };
    const post = (who, body, reply_to = null) => api(by[who], 'POST', `/api/rooms/${room.id}/messages`, { body, reply_to, attachments: [] });
    const longList = ['packing list for the trip, so nobody forgets anything this time:', ...['tent', 'two sleeping bags', 'the big cooler', 'camp stove and gas', 'water filter', 'head torches', 'first aid kit', 'rain jackets', 'spare socks', 'the card games', 'speaker and the good cable', 'power bank', 'sunscreen', 'bug spray', 'map, the paper one', 'rope', 'bin bags', 'coffee', 'mugs', 'a lighter', 'snacks for the drive', 'chairs'].map((x) => `- ${x}`)].join('\n');
    const m = [];
    m.push(await post('b', 'morning! the patch notes are out'));
    m.push(await post('b', 'link to the patch notes: https://example.com/notes'));
    m.push(await post('a', 'nice, reading them now'));
    m.push(await post('a', longList));
    m.push(await post('b', 'that is a **lot** of rope'));
    m.push(await post('a', 'anyone up for a game tonight?'));
    m.push(await post('b', 'I can do 9'));
    m.push(await post('b', 'after dinner works better for me'));
    m.push(await post('a', 'this is the config I use for voice:\n```\nbitrate = 64\nnoise = deepfilter\n```\nthe `deepfilter` mode helps a lot'));
    m.push(await post('b', 'count me in', m[5].id));
    const mic = await post('a', 'I fixed my mic, tell me if it still crackles');
    m.push(mic);
    await api(alice.page, 'PATCH', `/api/messages/${mic.id}`, { body: 'I fixed my mic, tell me if it still crackles at all' });
    const gone = await post('b', 'wrong room, sorry');
    m.push(gone);
    await api(bob.page, 'DELETE', `/api/messages/${gone.id}`);
    await api(alice.page, 'PUT', `/api/rooms/${room.id}/read`, { last_read_id: gone.id });
    m.push(await post('b', '@alice can you host? my upload is slow'));
    const coop = await post('b', 'the co-op one again?');
    m.push(coop);
    for (const [who, e] of [['a', '👍'], ['b', '👍'], ['b', '😂']]) await api(by[who], 'PUT', `/api/messages/${coop.id}/reaction`, { emoji: e, on: true });
    const plan = await post('b', 'the plan: 9pm, co-op, bring snacks');
    m.push(plan);
    await api(bob.page, 'PUT', `/api/messages/${plan.id}/pin`, { on: true });
    // Days: the first five two days back, the next three yesterday, the rest today.
    const shift = new Map(m.slice(0, 5).map((x) => [x.id, 2]).concat(m.slice(5, 8).map((x) => [x.id, 1])));
    return { room, shift };
  })();
  return planned;
}
// Rewrites history pages so the shifted messages look older; `limit` makes pages small (to show "Load older").
async function backdate(page, shift, { limit, holdOlder } = {}) {
  await page.route('**/api/rooms/*/messages?*', async (route) => {
    const url = new URL(route.request().url());
    if (route.request().method() !== 'GET') return route.continue();
    if (limit) url.searchParams.set('limit', String(limit));
    if (holdOlder && url.searchParams.has('before')) await holdOlder;
    const res = await route.fetch({ url: url.href });
    const body = await res.json();
    for (const x of body.messages) x.created_at -= (shift.get(x.id) ?? 0) * 86_400_000;
    await route.fulfill({ response: res, json: body });
  });
}
async function openPlans(ctx, opts, page = ctx.alice.page) {
  await ctx.seed();
  const { shift } = await plans(ctx);
  await backdate(page, shift, opts);
  await page.goto(`${ORIGIN}/`);
  await signedIn(page);
  await openRoom(page, 'plans');
  await page.locator('article').first().waitFor();
  return page;
}
scene('timeline', async (ctx) => {
  const page = await openPlans(ctx);
  await page.mouse.move(1439, 899);
  await sleep(400);
  await ctx.shot(page, 'timeline');
  // The New row is in the last screenful, so the room opens at the bottom: no "Jump to latest".
  if (await page.getByRole('button', { name: 'Jump to latest' }).count()) throw new Error('timeline: "Jump to latest" shows at the bottom');
  // Read: the New row fades, then folds away with no gap left, and the view stays at the bottom.
  await page.waitForFunction(() => document.querySelector('[data-timeline] > .new')?.getBoundingClientRect().height === 0, null, { timeout: 10_000 });
  await sleep(400);
  const gap = await page.locator('[data-timeline]').evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight);
  if (gap >= 8) throw new Error(`timeline-read: ${gap} px from the bottom after the New row folded`);
  await ctx.shot(page, 'timeline-read');
  // The toolbar on hover over a grouped message, and on keyboard focus (Tab into the row) with its More menu open.
  await page.locator('article', { hasText: 'after dinner works better' }).hover();
  await sleep(200);
  await ctx.shot(page, 'timeline-hover');
  const own = page.locator('article', { hasText: 'I fixed my mic' });
  await own.getByRole('button', { name: 'More', exact: true }).focus();
  await page.mouse.move(1439, 899);
  await page.keyboard.press('Enter');
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).waitFor();
  await sleep(300);
  await ctx.shot(page, 'timeline-menu');
  await page.keyboard.press('Escape');
  // Down from the toolbar: the next row takes focus (its ring and its toolbar show).
  await page.keyboard.press('ArrowDown');
  await sleep(300);
  await ctx.shot(page, 'timeline-row-focus');
  // A copy with no clipboard at all: the plain note at the bottom left.
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'clipboard', { value: undefined });
    document.execCommand = () => false;
  });
  await own.getByRole('button', { name: 'More', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Copy text', exact: true }).click();
  await page.getByText('Could not copy.', { exact: false }).waitFor();
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'timeline-copy-failed');
  // The long message expanded, scrolled into view.
  const long = page.locator('article', { hasText: 'packing list' });
  await long.evaluate((el) => (el.parentElement.scrollTop = el.offsetTop - 8)); // the list only, not the page
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'timeline-collapsed');
  await long.getByRole('button', { name: 'Show more' }).click();
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'timeline-expanded');
  // A reply quote jumps to the original and flashes it: the frame just after the click.
  await page.getByRole('button', { name: /^Reply to alice/ }).click();
  await page.mouse.move(1439, 899);
  await sleep(120);
  await ctx.shot(page, 'timeline-jump');
  await page.unrouteAll({ behavior: 'wait' }); // let a page in flight finish first
});
// Small pages: the room opens at the bottom with "N new messages. Show the first", and scrolling up shows the older
// page loading (held back until the picture is taken).
scene('timeline-older', async (ctx) => {
  // bob has never opened plans, so all of alice's messages there are unread for him.
  let release;
  const holdOlder = new Promise((r) => (release = r));
  const page = await openPlans(ctx, { limit: 6, holdOlder }, ctx.bob.page);
  await page.getByRole('button', { name: /new messages\. Show the first/ }).waitFor();
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'timeline-unread-pill');
  await page.getByRole('button', { name: 'Load older messages' }).click(); // six short messages do not fill the pane
  await page.locator('[aria-busy="true"]', { hasText: 'Load older messages' }).waitFor();
  await sleep(200);
  await ctx.shot(page, 'timeline-older');
  release();
  await page.unrouteAll({ behavior: 'wait' }); // let a page in flight finish first
});
scene('timeline-empty', async ({ alice, api }) => {
  await api(alice.page, 'POST', '/api/rooms', { name: 'quiet' });
  await alice.page.goto(`${ORIGIN}/`);
  await signedIn(alice.page);
  await openRoom(alice.page, 'quiet');
  await alice.page.getByText('No messages yet.').waitFor();
  await alice.page.mouse.move(1439, 899);
  await sleep(300);
  return alice.page;
});

// The composer in general, above the call bar (alice is in the music call): empty, a multi-line draft, the @ list with
// three people, the reply bar, Up editing the last message, two people typing, the formatting help, a failed send and
// the counter past the limit.
scene('composer', async (ctx) => {
  await fullHouse(ctx);
  const page = ctx.alice.page;
  const box = page.getByRole('textbox', { name: 'Message', exact: true });
  const still = async () => { await page.mouse.move(1439, 899); await sleep(300); };
  await openRoom(page, 'general');
  await rest(page);
  await ctx.shot(page, 'composer-empty');
  await box.fill('three things for friday:\nbring the **good** cable\nsomeone check the `deepfilter` mode\n> last time it crackled\nand snacks');
  await still();
  await ctx.shot(page, 'composer-multiline');
  await box.fill('');
  await box.pressSequentially('can someone check @a');
  await page.getByRole('option').nth(2).waitFor();
  await still();
  await ctx.shot(page, 'composer-mention');
  await box.press('Escape');
  await box.fill('sure, I can bring it');
  const theirs = page.locator('article', { hasText: 'sounds fine to me' }).last();
  await theirs.hover();
  await theirs.getByRole('button', { name: 'Reply', exact: true }).click();
  await still();
  await ctx.shot(page, 'composer-reply');
  await box.press('Escape');
  await box.fill('');
  await box.press('ArrowUp');
  await page.getByLabel('Edit message').waitFor();
  await still();
  await ctx.shot(page, 'composer-edit-last');
  await page.getByLabel('Edit message').press('Escape');
  for (const p of [ctx.bob.page, extra[0].page]) {
    await openRoom(p, 'general');
    await p.getByRole('textbox', { name: 'Message', exact: true }).pressSequentially('on my way');
  }
  await page.getByText('bob and carol are typing').waitFor({ timeout: 10000 });
  await rest(page);
  await ctx.shot(page, 'composer-typing');
  for (const p of [ctx.bob.page, extra[0].page]) await p.getByRole('textbox', { name: 'Message', exact: true }).fill('');
  await page.getByRole('button', { name: 'Formatting help' }).click();
  await still();
  await ctx.shot(page, 'composer-help');
  await page.keyboard.press('Escape');
  await page.route('**/api/rooms/*/messages', (r) => r.request().method() === 'POST' ? r.fulfill({ status: 429, json: { code: 'rate_limited', message: 'slow down' } }) : r.fallback());
  await box.fill('see you at nine');
  await box.press('Enter');
  await page.getByRole('alert').filter({ hasText: 'Too many tries' }).waitFor();
  await still();
  await ctx.shot(page, 'composer-error');
  await page.unrouteAll({ behavior: 'wait' });
  await box.fill('a long story. '.repeat(287)); // 4018 characters, 4017 after the trim
  await still();
  await ctx.shot(page, 'composer-limit');
  await box.fill('');
});
// The header's pins list with three pins in plans, then a room with none.
scene('pins', async (ctx) => {
  await ctx.seed();
  const { room } = await plans(ctx);
  // Pinned before the room opens: a pin pushes the message with its real time, which would undo the harness's backdating.
  const msgs = (await ctx.api(ctx.alice.page, 'GET', `/api/rooms/${room.id}/messages?limit=50`)).messages;
  for (const body of ['anyone up for a game tonight?', 'I can do 9']) {
    const m = msgs.find((x) => x.body === body);
    if (!m.pinned) await ctx.api(ctx.alice.page, 'PUT', `/api/messages/${m.id}/pin`, { on: true });
  }
  const page = await openPlans(ctx);
  await page.getByRole('button', { name: 'Pins', exact: true }).click();
  await page.getByRole('dialog', { name: 'Pinned messages' }).getByText('of 50 pins').waitFor();
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'pins');
  await page.keyboard.press('Escape');
  await page.unrouteAll({ behavior: 'wait' });
  const rooms = await ctx.api(page, 'GET', '/api/rooms');
  if (!rooms.some((r) => r.name === 'nopins')) await ctx.api(page, 'POST', '/api/rooms', { name: 'nopins' });
  await openRoom(page, 'nopins');
  await page.getByRole('button', { name: 'Pins', exact: true }).click();
  await page.getByText('Nothing is pinned yet.').waitFor();
  await page.mouse.move(1439, 899);
  await sleep(300);
  await ctx.shot(page, 'pins-empty');
  await page.keyboard.press('Escape');
});

// Uploads: a room "photos" with a wide and a tall image and a file card. `png(w, h)` draws a soft two-colour gradient
// (no binary in the repo). The upload scenes swap XMLHttpRequest for a fake that holds each upload at a set point.
function png(w, h, [r0, g0, b0], [r1, g1, b1]) {
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) {
    const row = y * (w * 3 + 1);
    for (let x = 0; x < w; x++) {
      const t = (x / w + y / h) / 2;
      raw.set([r0 + (r1 - r0) * t, g0 + (g1 - g0) * t, b0 + (b1 - b0) * t].map(Math.round), row + 1 + x * 3);
    }
  }
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const out = Buffer.alloc(12 + data.length);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(crc32(body), 8 + data.length);
    return out;
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2; // RGB
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}
let photos;
function photosRoom({ alice, bob, api }) {
  photos ??= (async () => {
    const room = await api(alice.page, 'POST', '/api/rooms', { name: 'photos' });
    const up = (page, name, buf) => page.evaluate(async ([room, name, b64]) => {
      const r = await fetch(`/api/rooms/${room}/uploads?name=${encodeURIComponent(name)}`, { method: 'POST', body: Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)) });
      return (await r.json()).attachment.id;
    }, [room.id, name, buf.toString('base64')]);
    const post = (page, body, attachments = []) => api(page, 'POST', `/api/rooms/${room.id}/messages`, { body, reply_to: null, attachments });
    await post(bob.page, 'who has the photos from saturday?');
    await post(alice.page, 'here, the view from the top', [await up(alice.page, 'summit-panorama-saturday-evening.png', png(1200, 600, [40, 52, 84], [214, 140, 72]))]);
    await post(bob.page, 'and the tower on the way down', [await up(bob.page, 'tower.png', png(420, 900, [92, 110, 96], [30, 34, 40]))]);
    await post(alice.page, 'the route notes too', [await up(alice.page, 'route-notes.pdf', Buffer.from('%PDF-1.4\n% route notes\n'))]);
    await post(bob.page, 'great, thanks');
    return room;
  })();
  return photos;
}
async function openPhotos(ctx) {
  await photosRoom(ctx);
  const page = ctx.alice.page;
  await page.goto(`${ORIGIN}/`);
  await signedIn(page);
  await openRoom(page, 'photos');
  await page.locator('article', { hasText: 'great, thanks' }).waitFor();
  return page;
}
// The timeline while the images are still on their way (the boxes are kept), then loaded (the view did not move), the
// viewer on the panorama, then the next image with the arrow key.
scene('message-image', async (ctx) => {
  const page = ctx.alice.page;
  let release;
  const gate = new Promise((r) => (release = r));
  await page.route('**/api/uploads/*', async (r) => { await gate; await r.continue(); });
  await openPhotos(ctx);
  await rest(page);
  const top = () => page.locator('article', { hasText: 'great, thanks' }).evaluate((el) => el.getBoundingClientRect().top);
  const before = await top();
  await ctx.shot(page, 'message-image-loading');
  release();
  await page.waitForFunction(() => [...document.querySelectorAll('article button img')].every((i) => i.complete && i.naturalWidth));
  await sleep(300);
  await page.unrouteAll({ behavior: 'wait' });
  console.log(`message-image: the last row moved ${(await top()) - before} px (at the bottom, so it follows the bottom)`);
  await ctx.shot(page, 'message-image');
  await page.getByRole('button', { name: 'Open image summit-panorama-saturday-evening.png' }).click();
  await page.getByRole('dialog').getByText('1 of 2').waitFor();
  await page.mouse.move(1439, 899);
  await sleep(400);
  await ctx.shot(page, 'image-viewer');
  await page.keyboard.press('ArrowRight');
  await page.getByRole('dialog').getByText('2 of 2').waitFor();
  await sleep(300);
  await ctx.shot(page, 'image-viewer-next');
  await page.keyboard.press('Escape');
});
// Files dragged over the room.
scene('drop-overlay', async (ctx) => {
  const page = await openPhotos(ctx);
  const dt = await page.evaluateHandle(() => { const d = new DataTransfer(); d.items.add(new File(['x'], 'a.png', { type: 'image/png' })); return d; });
  await page.getByRole('region', { name: 'Chat' }).dispatchEvent('dragenter', { dataTransfer: dt });
  await page.getByText('Drop to attach').waitFor();
  await rest(page);
  await ctx.shot(page, 'drop-overlay');
  await page.getByRole('region', { name: 'Chat' }).dispatchEvent('dragleave', { dataTransfer: dt });
});
// The upload tray with a fake XMLHttpRequest: a 5 MB file at 40 percent alone, then the mix (one failed with Retry, three
// uploading, one waiting), then the failures (too large, quota, rate limit, network).
async function fakeUploads(page) {
  await page.evaluate(() => {
    const at = { 'clip.mp4': 0.4, 'photo.png': 0.72, 'scan.png': 0.15 };
    const fail = { 'backup.zip': [507, 'quota_full'], 'scan-2.png': [429, 'rate_limited'] };
    window.XMLHttpRequest = class {
      upload = {};
      open(_m, url) { this.name = new URL(url, location.href).searchParams.get('name'); }
      abort() { this.onabort?.(); }
      send() {
        setTimeout(() => {
          if (this.name === 'notes.txt') return this.onerror?.();
          if (fail[this.name]) {
            [this.status, this.responseText] = [fail[this.name][0], JSON.stringify({ code: fail[this.name][1] })];
            return this.onload?.();
          }
          this.upload.onprogress?.({ lengthComputable: true, loaded: (at[this.name] ?? 0.5) * 100, total: 100 });
        }, 30);
      }
    };
  });
}
scene('upload-tray', async (ctx) => {
  const page = await openPhotos(ctx);
  await fakeUploads(page);
  const input = page.getByLabel('Attach a file');
  const f = (name, buffer, mimeType = 'application/octet-stream') => ({ name, mimeType, buffer });
  await input.setInputFiles([f('clip.mp4', Buffer.alloc(5 * 1024 * 1024), 'video/mp4')]);
  await page.getByRole('progressbar').waitFor();
  await sleep(300);
  await rest(page);
  await ctx.shot(page, 'upload-progress');
  await page.getByRole('button', { name: 'Cancel clip.mp4' }).click();
  await input.setInputFiles([f('backup.zip', Buffer.alloc(180 * 1024))]);
  await page.getByRole('alert').filter({ hasText: 'no room for more files' }).waitFor();
  await input.setInputFiles([
    f('clip.mp4', Buffer.alloc(5 * 1024 * 1024), 'video/mp4'),
    f('photo.png', png(64, 48, [214, 140, 72], [40, 52, 84]), 'image/png'),
    f('scan.png', png(48, 64, [92, 110, 96], [230, 230, 220]), 'image/png'),
    f('song-with-a-rather-long-file-name-for-the-tray.flac', Buffer.alloc(2 * 1024 * 1024), 'audio/flac'),
  ]);
  await page.getByText('Waiting', { exact: true }).waitFor();
  await sleep(300);
  await rest(page);
  await ctx.shot(page, 'upload-tray');
  await page.setViewportSize({ width: 1024, height: 900 });
  await sleep(300);
  await ctx.shot(page, 'upload-tray-narrow');
  await page.setViewportSize(VIEWPORT);
  // Another room while they upload: its composer says where the uploads are, and the files are there on the way back.
  await openRoom(page, 'general');
  await page.getByText('Uploading in photos').waitFor();
  await rest(page);
  await ctx.shot(page, 'upload-elsewhere');
  await openRoom(page, 'photos');
  await page.getByText('Waiting', { exact: true }).waitFor();
  for (const name of ['backup.zip', 'clip.mp4', 'photo.png', 'scan.png', 'song-with-a-rather-long-file-name-for-the-tray.flac']) {
    await page.getByRole('button', { name: new RegExp(`^(Cancel|Remove) ${name.replace('.', '\\.')}$`) }).click();
  }
  await input.setInputFiles([
    f('raw-footage.mov', Buffer.alloc(25 * 1024 * 1024 + 1), 'video/quicktime'),
    f('backup.zip', Buffer.alloc(180 * 1024)),
    f('scan-2.png', png(48, 64, [92, 110, 96], [230, 230, 220]), 'image/png'),
    f('notes.txt', Buffer.from('some notes'), 'text/plain'),
  ]);
  await page.getByRole('alert').filter({ hasText: 'connection' }).waitFor();
  await sleep(300);
  await rest(page);
  await ctx.shot(page, 'upload-errors');
  await page.reload(); // the real XMLHttpRequest again
  await signedIn(page);
});

// The search dialog over plans (search hits get the same day shift as the timeline, so they group by day).
// `answer` fakes the server's reply: 'hold' keeps it pending (the loading line), 429 is the rate limit (busy after the
// one retry a second later).
async function openSearch(ctx, text, { answer } = {}) {
  const page = await openPlans(ctx);
  const { shift } = await plans(ctx);
  let release;
  const held = new Promise((r) => (release = r));
  await page.route('**/api/search?*', async (route) => {
    if (answer === 429) return route.fulfill({ status: 429, json: { code: 'rate_limited' } });
    if (answer === 'hold') await held;
    const res = await route.fetch();
    const body = await res.json();
    for (const h of body.hits) h.message.created_at -= (shift.get(h.message.id) ?? 0) * 86_400_000;
    body.hits.sort((x, y) => y.message.created_at - x.message.created_at); // newest first, as the server's ids would be
    await route.fulfill({ response: res, json: body });
  });
  await rest(page);
  await page.keyboard.press('Control+Shift+F');
  const box = page.getByRole('combobox', { name: 'Search messages' });
  await box.waitFor();
  if (text) await box.fill(text);
  await sleep(answer === 429 ? 2000 : 900); // past the debounce and the answer (and the 429's retry)
  return { page, release };
}
const day2 = new Date(Date.now() - 2 * 86_400_000);
const iso = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
for (const [name, text, answer] of [
  ['search-empty', ''],
  ['search-results', 'patch'],
  ['search-filters', `from:bob in:plans after:${iso(day2)} the`],
  ['search-suggest', 'from:b'],
  ['search-none', 'zebra'],
  ['search-unknown', 'from:carl hello'],
  ['search-hint', 'p'],
  ['search-busy', 'patch', 429],
  ['search-dates', `after:${iso(new Date())} before:${iso(day2)} patch`],
  ['search-long', 'patch ' + 'plans for the weekend '.repeat(500)],
  ['search-loading', 'patch', 'hold'],
])
  scene(name, async (ctx) => {
    const { page, release } = await openSearch(ctx, text, { answer });
    await ctx.shot(page, name);
    release();
    await page.keyboard.press('Escape');
    await page.unrouteAll({ behavior: 'wait' });
  });
// A hit focused from the keyboard (Down from the box): its row and the Jump hint show.
scene('search-focus', async (ctx) => {
  const { page } = await openSearch(ctx, 'game');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowDown');
  await sleep(200);
  await ctx.shot(page, 'search-focus');
  await page.keyboard.press('Escape');
  await page.unrouteAll({ behavior: 'wait' });
});
scene('room', async ({ alice, bob, seed }) => {
  await seed();
  for (const p of [alice.page, bob.page]) {
    await home(p);
    await openRoom(p, 'general'); // `/` opens the last room, which an earlier scene may have changed
    await joinVoice(p);
  }
  await inVoice(alice.page, 'bob').waitFor();
  await sleep(1000);
  return alice.page;
});

// Each Settings page at 1440x900; Voice also whole (a tall window, as the page scrolls inside the pane) and at 1024x640.
async function settingsPage(page, name) {
  const nav = page.getByRole('navigation', { name: 'Settings sections' });
  if (!(await nav.count())) await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await nav.getByRole('link', { name }).click();
  await page.getByRole('heading', { level: 2, name }).waitFor();
}
async function tall(page, ctx, name) {
  const h = await page.locator('main .max-w-160').evaluate((e) => e.scrollHeight + 200);
  await page.setViewportSize({ width: VIEWPORT.width, height: Math.max(VIEWPORT.height, h) });
  await rest(page);
  await ctx.shot(page, name);
  await page.setViewportSize(VIEWPORT);
}
scene('settings', async (ctx) => {
  const page = ctx.alice.page;
  await home(page);
  await settingsPage(page, 'Voice');
  await page.getByLabel('Push to talk').check();
  await page.getByRole('button', { name: 'Mic test (hear yourself)' }).click();
  await page.waitForFunction(() => Number(document.querySelector('[role=meter]')?.getAttribute('aria-valuenow')) > -60);
  await rest(page);
  await ctx.shot(page, 'settings-voice');
  await tall(page, ctx, 'settings-voice-full');
  await page.setViewportSize({ width: 1024, height: 640 });
  await rest(page);
  await ctx.shot(page, 'settings-voice-narrow');
  await page.setViewportSize(VIEWPORT);
  await page.getByRole('button', { name: 'Stop mic test' }).click();
  // The key capture: waiting, then a refused Control.
  const key = page.getByRole('button', { name: /^Key:/ });
  await key.scrollIntoViewIfNeeded();
  await key.click();
  await page.mouse.move(1439, 899);
  await sleep(200);
  await ctx.shot(page, 'settings-ptt-capturing', { fullPage: false });
  await page.keyboard.press('Control');
  await page.getByText('Pick a key that is not Ctrl, Alt, Shift or Meta.').waitFor();
  await ctx.shot(page, 'settings-ptt-refused', { fullPage: false });
  await page.keyboard.press('Escape');
  // The reset confirm for Talk mode; Cancel keeps the setting.
  await page.getByRole('group', { name: 'Talk mode' }).getByRole('button', { name: 'Reset to defaults' }).click();
  await page.getByRole('dialog').waitFor();
  await sleep(300);
  await ctx.shot(page, 'settings-reset-confirm', { fullPage: false });
  await page.getByRole('button', { name: 'Cancel' }).click();
  await page.getByLabel('Push to talk').uncheck();
  // Headless Chromium answers 'denied' without asking: show the not-asked state with the getter overridden on this page.
  await page.evaluate(() => Object.defineProperty(Notification, 'permission', { get: () => 'default', configurable: true }));
  for (const [name, file] of [['Notifications', 'settings-notifications'], ['Appearance', 'settings-appearance']]) {
    await settingsPage(page, name);
    await rest(page);
    await ctx.shot(page, file);
  }
  // Notifications allowed (the browser grants here without a prompt), then a mention from bob for the title count.
  await page.evaluate(() => delete Notification.permission);
  await page.context().grantPermissions(['notifications'], { origin: ORIGIN });
  await settingsPage(page, 'Notifications');
  await page.getByRole('switch', { name: 'Notify me when someone mentions me' }).check();
  await page.getByRole('status').filter({ hasText: 'Allowed' }).waitFor();
  await rest(page);
  await ctx.shot(page, 'settings-notifications-allowed');
  const general = (await ctx.api(page, 'GET', '/api/rooms')).find((r) => r.name === 'general');
  await ctx.api(ctx.bob.page, 'POST', `/api/rooms/${general.id}/messages`, { body: '@alice can you check this' });
  await ctx.api(ctx.bob.page, 'POST', `/api/rooms/${general.id}/messages`, { body: '@alice and this one' });
  await page.waitForFunction(() => document.title === '(2) Vesper');
  console.log(`title-count: document.title is "${await page.evaluate(() => document.title)}"`);
  await page.getByRole('switch', { name: 'Notify me when someone mentions me' }).uncheck();
  await page.context().clearPermissions();
  // Account, whole page, with a second device (a fresh browser signed in as alice) and a wrong current password.
  const second = await ctx.open(`${ORIGIN}/login`);
  await second.getByLabel('Username').fill('alice');
  await second.getByLabel('Password', { exact: true }).fill('test-password-1');
  await second.getByRole('button', { name: 'Sign in' }).click();
  await signedIn(second);
  await settingsPage(page, 'Account');
  await page.getByRole('list', { name: 'Signed-in devices' }).getByRole('listitem').nth(1).waitFor();
  await page.getByLabel('Current password', { exact: true }).fill('test-password-9');
  await page.getByLabel('New password', { exact: true }).fill('test-password-2');
  await page.getByLabel('Confirm new password', { exact: true }).fill('test-password-2');
  await page.getByRole('button', { name: 'Change password' }).click();
  await page.getByText('Your current password is not right.').waitFor();
  await tall(page, ctx, 'settings-account');
  await page.getByRole('list', { name: 'Signed-in devices' }).getByRole('listitem').filter({ hasNotText: 'This device' }).getByRole('button', { name: 'Sign out' }).click();
  await second.getByText('Your session ended. Sign in again.').waitFor();
});
// The meter at three levels, from the primitives gallery.
scene('settings-meter', async ({ open }) => {
  const page = await open(`${await devServer()}/_gallery?primitives`);
  const m = page.locator('[data-meters]');
  await m.scrollIntoViewIfNeeded();
  await m.screenshot({ path: `${out}settings-meter.png` });
  console.log(`${out}settings-meter.png`);
});
scene('settings-voice-in-call', async (ctx) => {
  const page = ctx.alice.page;
  await home(page);
  await joinVoice(page);
  await settingsPage(page, 'Voice');
  await page.waitForFunction(() => Number(document.querySelector('[role=meter]')?.getAttribute('aria-valuenow')) > -100);
  await tall(page, ctx, 'settings-voice-in-call');
  await settingsPage(page, 'Diagnostics');
  await sleep(2500); // the stats refresh
  await rest(page);
  await ctx.shot(page, 'settings-diagnostics');
  await page.getByRole('button', { name: 'Close' }).click();
  await page.getByRole('button', { name: 'Leave' }).click();
});

// Main-thread CPU on bob's page in a two-person call: CDP TaskDuration over 10 s, printed, no picture. Run it three times
// and take the median (docs/dev.md has the baseline).
scene('cpu-baseline', async ({ alice, bob }) => {
  for (const p of [alice.page, bob.page]) {
    await home(p);
    await openRoom(p, 'general'); // `/` opens the last room, which an earlier scene may have changed
    await joinVoice(p);
  }
  await inVoice(bob.page, 'alice').waitFor();
  await sleep(2000); // let the call settle
  const cdp = await bob.page.context().newCDPSession(bob.page);
  await cdp.send('Performance.enable');
  const task = async () => (await cdp.send('Performance.getMetrics')).metrics.find((m) => m.name === 'TaskDuration').value;
  const t0 = await task();
  await sleep(10000);
  console.log(`cpu-baseline: ${Math.round((await task() - t0) * 1000)} ms per 10 s`);
});

// Offline: the composer keeps the draft and says Send waits for the connection. Restarts the server like reconnecting.
scene('composer-offline', async ({ alice, server, shot }) => {
  await home(alice.page);
  const box = alice.page.getByRole('textbox', { name: 'Message', exact: true });
  await box.fill('is the server down again?');
  await server.restart(async () => {
    await alice.page.getByText(/You are offline/).waitFor({ timeout: 10000 });
    await alice.page.mouse.move(1439, 899);
    await sleep(300);
    await shot(alice.page, 'composer-offline');
  });
  await alice.page.getByRole('button', { name: 'Try now' }).click().catch(() => {});
  await alice.page.getByText('Connection lost').waitFor({ state: 'detached', timeout: 30000 });
  await box.fill('');
});

// The server goes away: the banner counts down to the next try; it comes back and the banner goes. Runs last.
scene('reconnecting', async ({ alice, server, shot }) => {
  await home(alice.page);
  await server.restart(async () => {
    await alice.page.getByText(/Connection lost\. Trying again in \d+ s\./).waitFor({ timeout: 10000 });
    await rest(alice.page);
    await shot(alice.page, 'reconnecting');
  });
  await alice.page.getByRole('button', { name: 'Try now' }).click().catch(() => {}); // it may have reconnected already
  await alice.page.getByText('Connection lost').waitFor({ state: 'detached', timeout: 30000 });
});

const LINES = [
  'morning all', 'anyone up for a game tonight?', 'I can do 9', 'same, after dinner', 'what are we playing',
  'the co-op one again?', 'sure', 'I fixed my mic, tell me if it still crackles', 'sounds fine to me', 'link to the patch notes: https://example.com/notes',
];

const GROUPS = { uploads: ['message-image', 'drop-overlay', 'upload-tray'] }; // npm run shots -- uploads
const names = process.argv.slice(2).flatMap((n) => GROUPS[n] ?? [n]);
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
  let seeded;
  const seed = () => (seeded ??= (async () => {
    for (const name of ['music', LONG_ROOM]) await api(alice.page, 'POST', '/api/rooms', { name });
    const general = (await api(alice.page, 'GET', '/api/rooms')).find((r) => r.name === 'general');
    for (let i = 0; i < 40; i++) {
      const from = i % 3 === 2 ? bob : alice;
      await api(from.page, 'POST', `/api/rooms/${general.id}/messages`, { body: LINES[i % LINES.length] });
    }
  })());

  const shot = async (page, name, { fullPage = true } = {}) => {
    const path = `${out}${name}.png`;
    await page.screenshot({ path, fullPage });
    console.log(path);
  };
  for (const name of run) {
    const page = await scenes.get(name)({ alice, bob, api, open, shot, seed, server: stop });
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
