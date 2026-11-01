// Screenshot scenes for review: node e2e/shots.mjs [names...] (npm run shots -- login room). All scenes when none are named.
// Starts the release server like the e2e tests (needs ../target/release/vesper-server and dist/), signs up the owner alice
// and the member bob, then writes e2e/shots/<name>.png at 1440x900, full page. Scenes that need content call seed() (two more
// rooms and 40 messages in general), so shell-empty runs before it; reconnecting restarts the server, so it runs last.
// The `tokens` and `gallery` scenes shoot /_gallery from a Vite dev server on a spare port, since the build leaves it out.
import { spawn } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
import { createServer } from 'node:net';
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

scene('settings', async ({ alice }) => {
  await home(alice.page);
  await alice.page.getByRole('button', { name: 'Settings', exact: true }).click();
  await alice.page.getByLabel('Audio settings').waitFor();
  await rest(alice.page);
  return alice.page;
});

scene('diagnostics', async ({ alice }) => {
  await home(alice.page);
  await joinVoice(alice.page);
  await alice.page.getByRole('button', { name: 'Settings', exact: true }).click();
  await alice.page.getByRole('link', { name: 'Diagnostics' }).click();
  await sleep(2500); // the stats refresh
  return alice.page;
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
