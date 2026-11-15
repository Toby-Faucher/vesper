// Desktop against web, picture by picture: node e2e/parity.mjs [scenes...] (all when none are named).
// One server (lib.mjs), the cast of shots.mjs (alice and bob), then per scene: the web app in Chromium at 1440x900, and the
// desktop app (a debug build: it reads the VESPER_DEV_* variables, src/ui/dev.rs) in a headless sway of its own at
// 1440x900, scale 1, signed in to the same server. Writes e2e/parity/<scene>-web.png, -desktop.png, -side.png (web left)
// and -diff.png (the web picture dimmed, differing pixels red), and prints the share of pixels that differ.
// A pixel differs when a channel is off by more than TOLERANCE (anti-aliasing and hinting stay under it).
// Needs ../target/release/vesper-server, dist/ and ../desktop/target/debug/vesper-desktop (desktop/scripts/parity.sh builds them).
// node e2e/parity.mjs --compare web.png desktop.png name: only the comparison, of two pictures already taken.
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ORIGIN, browser, inVoice, openRoom, signup, sleep, startServer, track } from './lib.mjs';
import { LONG_ROOM, VIEWPORT, api, backdate, cast, home, photosRoom, plans, rest, shotBrowser } from './shotlib.mjs';

const out = fileURLToPath(new URL('parity/', import.meta.url));
const app = process.env.VESPER_DESKTOP_BIN || fileURLToPath(new URL('../../desktop/target/debug/vesper-desktop', import.meta.url));
const TOLERANCE = 24;
const PASSWORD = 'test-password-1'; // what signup() in lib.mjs gives every account

const FRAMES = [60, 150];
const DRAFT = '## three things for friday\nbring the **good** cable, *please*\nsomeone check the `deepfilter` mode\n> last time it crackled\n- and ~~chips~~ snacks';
let typing;
const RING_FRAMES = [60, 150, 800];
const OFFLINE_DRAFT = 'is the server down again?';

// The upload tray's rows: name, size, MIME type, and the state the fake upload holds it in (dev.rs reads the same).
const TRAY = [
  ['backup.zip', 180 * 1024, 'application/octet-stream', 'failed:quota'],
  ['clip.mp4', 5 * 1024 * 1024, 'video/mp4', 'uploading:0.4'],
  ['photo.png', 2048, 'image/png', 'uploading:0.72'],
  ['scan.png', 2048, 'image/png', 'uploading:0.15'],
  ['song-with-a-rather-long-file-name-for-the-tray.flac', 2 * 1024 * 1024, 'audio/flac', 'queued'],
];
// web(ctx) returns the page to shoot (or null when it shot <scene>-web.png itself); desktop is the VESPER_DEV_* state, or an async function of ctx that returns it:
// `user` signs in, `scene` is sign-in, room:<name> or settings (src/ui/dev.rs), `settings` is written to the app's
// settings.json first, and `env` adds more VESPER_DEV_* variables (dev.rs lists them). `after` runs once both pictures
// are taken, and `focus` keeps the focus where the web steps left it. `offline` shoots the desktop while the server is
// stopped (VESPER_DEV_OFFLINE). Every desktop run gets the web scenes' clock (7:41 PM) and an en-US locale.
// The desktop has no passkeys, so the web login is shot without the passkey button.
const SCENES = {
  login: {
    web: async () => {
      const { page } = await shotBrowser('login', [], { viewport: VIEWPORT });
      await page.addInitScript(() => delete window.PublicKeyCredential);
      await page.goto(`${ORIGIN}/login`);
      await page.getByRole('button', { name: 'Sign in', exact: true }).waitFor();
      return page;
    },
    desktop: { scene: 'sign-in' },
  },
  'shell-empty': {
    web: async ({ alice }) => { await home(alice.page); return alice.page; },
    desktop: { user: 'alice' },
  },
  // general with the 40 seeded messages, all from the same minute.
  'timeline-general': {
    web: async ({ alice, seed }) => {
      await seed();
      await home(alice.page);
      await openRoom(alice.page, 'general');
      return alice.page;
    },
    desktop: { user: 'alice', scene: 'room:general' },
  },
  // shots.mjs's timeline: plans over three days (the older messages moved back a day or two on both sides), groups, a
  // code block, a reply, edited, pinned and deleted messages, reactions, the long message, a mention and the New row.
  // alice's read marker must stay where plans() put it for the desktop's New row, so the web's mark read never reaches
  // the server.
  timeline: {
    web: async (ctx) => {
      const page = await openPlans(ctx);
      return page;
    },
    desktop: async (ctx) => ({ user: 'alice', scene: 'room:plans', env: { VESPER_DEV_SHIFT: await shiftVar(ctx) } }),
    after: (ctx) => ctx.alice.page.unrouteAll({ behavior: 'wait' }),
  },
  'timeline-empty': {
    web: async ({ alice }) => {
      await api(alice.page, 'POST', '/api/rooms', { name: 'quiet' });
      await home(alice.page);
      await openRoom(alice.page, 'quiet');
      await alice.page.getByText('No messages yet.').waitFor();
      return alice.page;
    },
    desktop: { user: 'alice', scene: 'room:quiet', env: { VESPER_DEV_EMPTY: '1' } },
  },
  // The composer with a multi-line draft (shots.mjs's composer-multiline, without the call bar).
  composer: {
    web: async ({ alice, seed }) => {
      await seed();
      await home(alice.page);
      await openRoom(alice.page, 'general');
      await alice.page.getByRole('textbox', { name: 'Message', exact: true }).fill(DRAFT);
      return alice.page;
    },
    desktop: { user: 'alice', scene: 'room:general', env: { VESPER_DEV_DRAFT: DRAFT } },
    focus: true, // the box keeps focus, as after typing
    after: ({ alice }) => alice.page.getByRole('textbox', { name: 'Message', exact: true }).fill(''),
  },
  // The @ list with three people (carol, dave and Bartholomew from the full house).
  'composer-mention': {
    web: async (ctx) => {
      await house(ctx);
      const page = ctx.alice.page;
      await openRoom(page, 'general');
      const box = page.getByRole('textbox', { name: 'Message', exact: true });
      await box.pressSequentially('can someone check @a');
      await page.getByRole('option').nth(2).waitFor();
      return page;
    },
    desktop: { user: 'alice', scene: 'room:general', env: { VESPER_DEV_DRAFT: 'can someone check @a' } },
    focus: true, // a blur closes the @ list
    after: ({ alice }) => alice.page.getByRole('textbox', { name: 'Message', exact: true }).fill(''),
  },
  // bob typing in general: his picture, the dots and "bob is typing". He keeps typing until both pictures are taken.
  'composer-typing': {
    web: async (ctx) => {
      await ctx.seed();
      const page = ctx.alice.page;
      await home(page);
      await openRoom(page, 'general');
      await home(ctx.bob.page);
      await openRoom(ctx.bob.page, 'general');
      const box = ctx.bob.page.getByRole('textbox', { name: 'Message', exact: true });
      typing = setInterval(() => void box.pressSequentially('a').catch(() => {}), 2000);
      await box.pressSequentially('on my way');
      await page.getByText('bob is typing').waitFor({ timeout: 10000 });
      return page;
    },
    desktop: { user: 'alice', scene: 'room:general', env: { VESPER_DEV_TYPING: '1' } },
    after: async ({ bob }) => {
      clearInterval(typing);
      await bob.page.getByRole('textbox', { name: 'Message', exact: true }).fill('');
    },
  },
  // The header's pins list on plans with three pins (shots.mjs's pins).
  pins: {
    web: async (ctx) => {
      const { room } = await plans(ctx);
      const msgs = (await api(ctx.alice.page, 'GET', `/api/rooms/${room.id}/messages?limit=50`)).messages;
      for (const body of ['anyone up for a game tonight?', 'I can do 9']) {
        const m = msgs.find((x) => x.body === body);
        if (!m.pinned) await api(ctx.alice.page, 'PUT', `/api/messages/${m.id}/pin`, { on: true });
      }
      const page = await openPlans(ctx);
      await page.getByRole('button', { name: 'Pins', exact: true }).click();
      await page.getByRole('dialog', { name: 'Pinned messages' }).getByText('of 50 pins').waitFor();
      return page;
    },
    desktop: async (ctx) => ({ user: 'alice', scene: 'room:plans', env: { VESPER_DEV_SHIFT: await shiftVar(ctx), VESPER_DEV_PINS: '1' } }),
    after: async ({ alice }) => { await alice.page.keyboard.press('Escape'); await alice.page.unrouteAll({ behavior: 'wait' }); },
  },
  // photos: a wide and a tall image and a file card, loaded.
  'message-image': {
    web: async (ctx) => {
      await photosRoom(ctx);
      const page = ctx.alice.page;
      await home(page);
      await openRoom(page, 'photos');
      await page.locator('article', { hasText: 'great, thanks' }).waitFor();
      await page.waitForFunction(() => [...document.querySelectorAll('article button img')].every((i) => i.complete && i.naturalWidth));
      return page;
    },
    desktop: { user: 'alice', scene: 'room:photos' },
  },
  // The first picture of photos open in the viewer: its name, the picture at fit size, Previous, Next and "1 of 2".
  'image-viewer': {
    web: async (ctx) => {
      await photosRoom(ctx);
      const page = ctx.alice.page;
      await home(page);
      await openRoom(page, 'photos');
      await page.locator('article', { hasText: 'great, thanks' }).waitFor();
      await page.locator('article button img').first().click();
      const dialog = page.getByRole('dialog');
      await dialog.getByRole('link', { name: 'Download' }).waitFor();
      await page.waitForFunction(() => [...document.querySelectorAll('dialog img')].every((i) => i.complete && i.naturalWidth));
      await sleep(400);
      return page;
    },
    focus: true,
    desktop: { user: 'alice', scene: 'room:photos', env: { VESPER_DEV_VIEWER: '0' } },
    after: ({ alice }) => alice.page.keyboard.press('Escape'),
  },
  // shots.mjs's upload-tray: one failed with Retry, three uploading, one waiting. The web's uploads are held by a fake
  // XMLHttpRequest; the desktop's tray is filled by dev.rs with the same rows (no upload runs).
  'upload-tray': {
    web: async (ctx) => {
      await photosRoom(ctx);
      const page = ctx.alice.page;
      await home(page);
      await openRoom(page, 'photos');
      await page.locator('article', { hasText: 'great, thanks' }).waitFor();
      await fakeUploads(page);
      const input = page.getByLabel('Attach a file');
      const f = (name, size, mimeType = 'application/octet-stream') => ({ name, mimeType, buffer: Buffer.alloc(size) });
      await input.setInputFiles([f('backup.zip', 180 * 1024)]);
      await page.getByRole('alert').filter({ hasText: 'no room for more files' }).waitFor();
      await input.setInputFiles(TRAY.slice(1).map(([name, size, mime]) => f(name, size, mime)));
      await page.getByText('Waiting', { exact: true }).waitFor();
      await sleep(300);
      return page;
    },
    desktop: { user: 'alice', scene: 'room:photos', env: { VESPER_DEV_TRAY: TRAY.map((t) => t.join('|')).join(';') } },
    after: async ({ alice }) => { await alice.page.reload(); },
  },
  // A sent message rising in (`rise`: 6 px up and a fade, 220 ms): alice sends on the web, bob from the desktop, in
  // hangout. Frames at 60 and 150 ms: the web's by pausing its animations there, the desktop's by grim that long after
  // the message is in the list (VESPER_DEV_ARRIVED); grim takes a few ms itself, the printed times are the real ones.
  // The final pictures differ by design: the desktop's list also has bob's message.
  'motion-send': {
    web: async ({ alice }) => {
      const page = alice.page;
      await home(page);
      await openRoom(page, 'hangout');
      await sleep(1600); // past settle()
      const box = page.getByRole('textbox', { name: 'Message', exact: true });
      await box.fill('on my way');
      await box.press('Enter');
      const row = page.locator('article', { hasText: 'on my way' }).last();
      await row.waitFor();
      await page.mouse.move(1439, 899);
      for (const t of FRAMES) {
        await row.evaluate((el, t) => el.getAnimations().forEach((a) => { a.pause(); a.currentTime = t; }), t);
        await page.screenshot({ path: `${out}motion-send-web-${t}ms.png` });
      }
      await row.evaluate((el) => el.getAnimations().forEach((a) => a.finish()));
      return page;
    },
    desktop: { user: 'bob', scene: 'room:hangout', env: { VESPER_DEV_SEND: 'on my way' }, frames: FRAMES },
  },
  shell: {
    web: async (ctx) => { await house(ctx); await openRoom(ctx.alice.page, 'general'); return ctx.alice.page; },
    desktop: { user: 'alice', scene: 'room:general' },
  },
  // shots.mjs's call scenes. One account can be in one call: alice (or bob, or dave) is in the music call in the web
  // picture, leaves, and the desktop's same account joins it (fake devices, dev.rs). The others are the house's.
  'call-bar': {
    web: (ctx) => inCall(ctx),
    desktop: async ({ alice }) => { await hangUp(alice.page); return { user: 'alice', scene: 'room:general', env: { VESPER_DEV_VOICE: 'music' } }; },
  },
  // Push to talk on, the key held.
  'call-ptt': {
    web: async (ctx) => {
      const page = await inCall(ctx);
      await page.getByRole('button', { name: 'Settings', exact: true }).click();
      await page.getByLabel('Push to talk').check();
      await page.goBack();
      await page.getByText('Push to talk: hold Space').waitFor();
      await rest(page);
      await page.keyboard.down('Space');
      await bar(page).getByText('Talking').waitFor();
      await sleep(300);
      return page;
    },
    focus: true,
    desktop: async ({ alice }) => {
      const page = alice.page;
      await page.keyboard.up('Space');
      await page.getByRole('button', { name: 'Settings', exact: true }).click();
      await page.getByLabel('Push to talk').uncheck();
      await page.goBack();
      await hangUp(page);
      return {
        user: 'alice', scene: 'room:general', settings: { push_to_talk: true, ptt_key: 'Space' },
        env: { VESPER_DEV_VOICE: 'music', VESPER_DEV_PTT_HELD: '1' },
      };
    },
  },
  // bob muted by the owner: Mute is disabled and the notice is in the status line. Shot from bob, in music.
  'call-force-muted': {
    web: async (ctx) => {
      await house(ctx);
      // An earlier scene may have taken bob's page out of the call (a reload ends it).
      if (!(await bar(ctx.bob.page).count())) {
        await openRoom(ctx.bob.page, 'music');
        await rejoin(ctx.bob.page);
      }
      const id = (await api(ctx.bob.page, 'GET', '/api/me')).user.user_id;
      await api(ctx.alice.page, 'POST', `/api/users/${id}/force-mute`, { muted: true });
      await ctx.bob.page.getByText('A moderator muted you.').waitFor();
      return ctx.bob.page;
    },
    desktop: async ({ bob }) => {
      await hangUp(bob.page); // the force mute holds across the desktop's join
      return { user: 'bob', scene: 'room:music', env: { VESPER_DEV_VOICE: 'music' } };
    },
    after: async ({ alice, bob }) => {
      const id = (await api(bob.page, 'GET', '/api/me')).user.user_id;
      await api(alice.page, 'POST', `/api/users/${id}/force-mute`, { muted: false });
      await rejoin(bob.page);
    },
  },
  // A weak connection: 420 ms round trip (the web's getStats wrapped, the desktop's VESPER_DEV_RTT).
  'call-weak': {
    web: async (ctx) => {
      const page = await inCall(ctx);
      await page.evaluate(() => {
        const pc = window.__pcs.at(-1);
        const get = pc.getStats.bind(pc);
        pc.getStats = async () => new Map([...(await get()).entries()].map(([k, v]) => [k, v.type === 'candidate-pair' ? { ...v, currentRoundTripTime: 0.42 } : v]));
      });
      await bar(page).getByText('Weak connection: 420 ms delay').waitFor({ timeout: 10000 });
      return page;
    },
    desktop: async ({ alice }) => { await hangUp(alice.page); return { user: 'alice', scene: 'room:general', env: { VESPER_DEV_VOICE: 'music', VESPER_DEV_RTT: '420' } }; },
  },
  // The microphone ended mid-call.
  'call-mic-lost': {
    web: async (ctx) => {
      const page = await inCall(ctx);
      await page.evaluate(() => {
        const t = window.__pcs.at(-1).getSenders().find((x) => x.track?.kind === 'audio').track;
        t.stop();
        t.dispatchEvent(new Event('ended'));
      });
      await bar(page).getByRole('button', { name: 'Retry microphone' }).waitFor();
      return page;
    },
    desktop: async ({ alice }) => { await hangUp(alice.page); return { user: 'alice', scene: 'room:general', env: { VESPER_DEV_VOICE: 'music', VESPER_DEV_MIC_LOST: '1' } }; },
  },
  // bob opens alice's card from the call bar and turns her up to 140 %.
  'person-card': {
    web: async (ctx) => {
      await inCall(ctx);
      const page = ctx.bob.page;
      await bar(page).getByRole('button', { name: 'alice' }).click();
      const card = page.getByRole('dialog', { name: 'alice' });
      await card.getByRole('slider', { name: 'Volume for alice' }).fill('140');
      await card.getByText('Can distort').waitFor();
      await page.mouse.move(1439, 899);
      await sleep(300);
      return page;
    },
    focus: true,
    desktop: async ({ bob }) => {
      const card = bob.page.getByRole('dialog', { name: 'alice' });
      await card.getByRole('button', { name: 'Reset' }).click();
      await bob.page.keyboard.press('Escape');
      await hangUp(bob.page);
      return { user: 'bob', scene: 'room:music', env: { VESPER_DEV_VOICE: 'music', VESPER_DEV_PERSON: 'alice@bar', VESPER_DEV_VOLUME: '140' } };
    },
    after: async ({ alice, bob }) => { await rejoin(bob.page); await hangUp(alice.page); },
  },
  // dave, a moderator, opens carol's card from the sidebar: the voice actions.
  'person-card-mod': {
    web: async (ctx) => {
      await house(ctx);
      const users = await api(ctx.alice.page, 'GET', '/api/users');
      const dave = users.find((u) => u.username === 'dave');
      await api(ctx.alice.page, 'PATCH', `/api/users/${dave.user_id}/role`, { role: 'moderator' });
      const page = housePages.dave;
      await inVoice(page, 'carol').getByRole('button').click();
      const card = page.getByRole('dialog', { name: 'carol' });
      await card.getByRole('button', { name: 'Remove from voice' }).waitFor();
      await page.mouse.move(1439, 899);
      await sleep(300);
      return page;
    },
    focus: true,
    desktop: async () => {
      const page = housePages.dave;
      await page.keyboard.press('Escape');
      await hangUp(page);
      return { user: 'dave', scene: 'room:music', env: { VESPER_DEV_VOICE: 'music', VESPER_DEV_DEAFEN: '1', VESPER_DEV_PERSON: 'carol@side' } };
    },
    after: async () => {
      await rejoin(housePages.dave);
      await housePages.dave.getByRole('button', { name: 'Deafen', exact: true }).click();
    },
  },
  // The connection icons on the call's voice rows.
  'voice-quality': {
    web: async (ctx) => {
      const page = await inCall(ctx);
      for (const n of ['bob', 'carol', 'dave']) await inVoice(page, n).getByRole('img', { name: /^Connection to .*: (good|fair|poor)$/ }).waitFor({ timeout: 15000 });
      return page;
    },
    desktop: async ({ alice }) => { await hangUp(alice.page); return { user: 'alice', scene: 'room:general', env: { VESPER_DEV_VOICE: 'music', VESPER_DEV_LINKS: '1' } }; },
  },
  // Offline with a draft: the composer keeps it and says Send waits (the server restarts).
  'composer-offline': {
    web: async ({ alice, seed }) => {
      await seed(); // the desktop's room:general waits for messages
      const page = alice.page;
      await home(page);
      await openRoom(page, 'general');
      const box = page.getByRole('textbox', { name: 'Message', exact: true });
      await box.fill(OFFLINE_DRAFT);
      await stopServer(async () => {
        await page.getByText(/You are offline. Your text is kept here/).waitFor({ timeout: 10000 });
        await page.mouse.move(1439, 899);
        await sleep(300);
        await page.screenshot({ path: `${out}composer-offline-web.png` });
      });
      await back(page);
      await box.fill('');
      return null;
    },
    desktop: { user: 'alice', scene: 'room:general', env: { VESPER_DEV_DRAFT: OFFLINE_DRAFT }, offline: true },
  },
  // The server goes away: the banner counts down to the next try.
  reconnecting: {
    web: async ({ alice, seed }) => {
      await seed(); // the desktop's room:general waits for messages
      const page = alice.page;
      await home(page);
      await openRoom(page, 'general');
      await stopServer(async () => {
        await page.getByText(/Connection lost\. Trying again in \d+ s\./).waitFor({ timeout: 10000 });
        await rest(page);
        await page.screenshot({ path: `${out}reconnecting-web.png` });
      });
      await back(page);
      return null;
    },
    desktop: { user: 'alice', scene: 'room:general', offline: true },
  },
  // The ring prompt rising in, and its pulse: bob calls alice in their DM. Frames at 60 and 150 ms (`rise`) and at
  // 800 ms (half way through the 1.6 s pulse): the web's by pausing every animation there, the desktop's by grim that
  // long after the ring came (VESPER_DEV_ARRIVED).
  'motion-ring': {
    web: async (ctx) => {
      await ctx.seed();
      const page = ctx.alice.page;
      await home(page);
      await openRoom(page, 'general');
      await sleep(1600); // past settle()
      await ringAlice(ctx);
      const prompt = page.getByRole('alertdialog');
      await prompt.waitFor();
      await page.mouse.move(1439, 899);
      for (const t of RING_FRAMES) {
        await page.evaluate((t) => document.getAnimations().forEach((a) => { a.pause(); a.currentTime = t; }), t);
        await page.screenshot({ path: `${out}motion-ring-web-${t}ms.png` });
      }
      await page.evaluate(() => document.getAnimations().forEach((a) => a.play()));
      return page;
    },
    focus: true,
    desktop: async (ctx) => {
      await hangUp(ctx.bob.page);
      return { user: 'alice', scene: 'room:general', frames: RING_FRAMES, trigger: () => ringAlice(ctx) };
    },
    after: ({ bob }) => hangUp(bob.page),
  },
  // Ctrl+K over general in the full house: unread rooms first, people in voice first, then the actions. The rooms'
  // recent list is cleared on the web, as the desktop starts with none.
  switcher: {
    web: async (ctx) => {
      await house(ctx);
      const page = ctx.alice.page;
      await openRoom(page, 'music');
      await page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith('vesper.recent.')).forEach((k) => localStorage.removeItem(k)));
      await openRoom(page, 'general');
      await page.keyboard.press('Control+k');
      await page.getByRole('combobox', { name: 'Switch to' }).waitFor();
      await sleep(400);
      return page;
    },
    focus: true,
    desktop: { user: 'alice', scene: 'room:general', env: { VESPER_DEV_FIND: 'switcher:' } },
    after: ({ alice }) => alice.page.keyboard.press('Escape'),
  },
  // Message search for a word in general's seeded lines: four hits today, from alice and bob (with pictures).
  search: {
    web: async (ctx) => {
      await ctx.seed();
      const page = ctx.alice.page;
      await home(page);
      await openRoom(page, 'general');
      await page.keyboard.press('Control+Shift+F');
      const box = page.getByRole('combobox', { name: 'Search messages' });
      await box.waitFor();
      await box.fill('crackles');
      await page.getByText(/^\d+\+? messages?$/).first().waitFor();
      await page.waitForFunction(() => [...document.querySelectorAll('dialog img')].every((i) => i.complete));
      await sleep(400);
      return page;
    },
    focus: true,
    desktop: { user: 'alice', scene: 'room:general', env: { VESPER_DEV_FIND: 'search:crackles' } },
    after: ({ alice }) => alice.page.keyboard.press('Escape'),
  },
  'shell-collapsed': {
    web: async (ctx) => {
      await house(ctx);
      const page = ctx.alice.page;
      await openRoom(page, 'general');
      await page.getByRole('button', { name: 'Hide sidebar' }).click();
      await page.getByRole('button', { name: 'Show sidebar' }).waitFor();
      await sleep(400); // the slide
      return page;
    },
    desktop: { user: 'alice', scene: 'room:general', settings: { sidebar_collapsed: true } },
    // The later scenes look for the room list.
    after: async ({ alice }) => {
      await alice.page.getByRole('button', { name: 'Show sidebar' }).click();
      await alice.page.getByRole('button', { name: 'Hide sidebar' }).waitFor();
    },
  },
  ...pageScenes('settings', ['voice', 'notifications', 'appearance', 'account', 'desktop']),
  ...pageScenes('admin', ['invites', 'members', 'rooms', 'disk', 'server']),
};
// Admin > Disk use from fixed numbers on both sides, not this machine's disk: one bar under 80 %, one over (the warning
// colour). The web's GET is answered by Playwright, the desktop's by dev.rs (VESPER_DEV_DISK).
const DISK = { db_bytes: 4200000, upload_bytes: 4800000000, quota_bytes: 10737418240, fs_total: 62000000000, fs_free: 10000000000, warn: true };
SCENES['admin-disk'].route = ['**/api/admin/disk', DISK];
SCENES['admin-disk'].desktop.env = { VESPER_DEV_DISK: JSON.stringify(DISK) };

// Settings and Admin, one scene per section: alice (the owner) opens /<page>/<section> on the web, and the desktop opens
// the same section (dev.rs `settings:<section>`, `admin:<section>`). The lists come from the same server.
function pageScenes(name, list) {
  return Object.fromEntries(list.map((section) => [`${name}-${section}`, {
    web: async (ctx) => {
      await house(ctx);
      const page = ctx.alice.page;
      await home(page);
      const route = SCENES[`${name}-${section}`].route;
      if (route) await page.route(route[0], (r) => r.fulfill({ json: route[1] }));
      await page.goto(`${ORIGIN}/${name}/${section}`);
      await page.getByRole('heading', { level: 1 }).waitFor();
      await sleep(1500); // the lazy page and its lists
      if (route) await page.unroute(route[0]);
      return page;
    },
    desktop: { user: 'alice', scene: `${name}:${section}` },
  }]));
}

const bar = (page) => page.getByRole('region', { name: 'Call' });
// alice in the house's music call, reading general.
async function inCall(ctx) {
  await house(ctx);
  const page = ctx.alice.page;
  await home(page);
  await openRoom(page, 'music');
  await rejoin(page);
  await openRoom(page, 'general');
  await bar(page).locator('[data-speaking]').first().waitFor({ timeout: 10000 }).catch(() => {});
  return page;
}
async function rejoin(page) {
  await page.getByRole('button', { name: 'Join voice' }).click();
  await page.getByRole('button', { name: 'Leave' }).waitFor();
}
async function hangUp(page) {
  await bar(page).getByRole('button', { name: 'Leave' }).click();
  await bar(page).waitFor({ state: 'detached' });
}
// The page is online again (the banner gone).
async function back(page) {
  await page.getByRole('button', { name: 'Try now' }).click({ timeout: 2000 }).catch(() => {});
  await page.getByText('Connection lost').waitFor({ state: 'detached', timeout: 30000 });
  await page.getByText(/You are offline/).waitFor({ state: 'detached', timeout: 30000 });
}
// bob joins voice in his DM with alice, which rings her.
async function ringAlice({ alice, bob }) {
  const id = (await api(alice.page, 'GET', '/api/me')).user.user_id;
  const dm = await api(bob.page, 'POST', '/api/dms', { user_id: id });
  await bob.page.goto(`${ORIGIN}/r/${dm.id}`);
  await rejoin(bob.page);
}
let stopServer; // the server's restart(during), set at start

// shots.mjs's fake XMLHttpRequest: each upload holds at its share, backup.zip fails with the quota.
async function fakeUploads(page) {
  await page.evaluate(() => {
    const at = { 'clip.mp4': 0.4, 'photo.png': 0.72, 'scan.png': 0.15 };
    window.XMLHttpRequest = class {
      upload = {};
      open(_m, url) { this.name = new URL(url, location.href).searchParams.get('name'); }
      abort() { this.onabort?.(); }
      send() {
        setTimeout(() => {
          if (this.name === 'backup.zip') {
            [this.status, this.responseText] = [507, JSON.stringify({ code: 'quota_full' })];
            return this.onload?.();
          }
          this.upload.onprogress?.({ lengthComputable: true, loaded: (at[this.name] ?? 0.5) * 100, total: 100 });
        }, 30);
      }
    };
  });
}

// plans, opened in alice's page with the day shift and her mark read held back.
async function openPlans(ctx) {
  await ctx.seed();
  const { shift } = await plans(ctx);
  const page = ctx.alice.page;
  await backdate(page, shift);
  await page.route('**/api/rooms/*/read', (r) => (r.request().method() === 'PUT' ? r.fulfill({ status: 204 }) : r.fallback()));
  await home(page);
  await openRoom(page, 'plans');
  await page.locator('article').first().waitFor();
  return page;
}
// The same shift for the desktop: "id:days,id:days".
async function shiftVar(ctx) {
  const { shift } = await plans(ctx);
  return [...shift].map(([id, d]) => `${id}:${d}`).join(',');
}

// shots.mjs's full house without alice in the call (the call bar is its own scene): bob, carol (muted), dave (deafened)
// and Bartholomew in voice in music, unread counts on music and a mention in the long room. The browsers stay open.
let housed;
const housePages = {};
function house({ alice, bob, seed }) {
  housed ??= (async () => {
    await seed();
    const rooms = await api(alice.page, 'GET', '/api/rooms');
    const id = (name) => rooms.find((r) => r.name === name).id;
    const pages = [bob.page];
    for (const [name, display] of [['carol', 'carol'], ['dave', 'dave'], ['bart', 'Bartholomew Featherstonehaugh']]) {
      const { url } = await api(alice.page, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 });
      const b = await shotBrowser(name, [], { viewport: VIEWPORT });
      await signup(b.page, url, name, display);
      pages.push(b.page);
      housePages[name] = b.page;
    }
    for (const p of pages) {
      await home(p);
      await openRoom(p, 'music');
      await p.getByRole('button', { name: 'Join voice' }).click();
      await p.getByRole('button', { name: 'Leave' }).waitFor();
    }
    await pages[1].getByRole('button', { name: 'Mute', exact: true }).click();
    await pages[2].getByRole('button', { name: 'Deafen', exact: true }).click();
    await home(alice.page);
    await openRoom(alice.page, 'general');
    for (const body of ['set list for friday is up', 'who has the good speaker cable']) await api(bob.page, 'POST', `/api/rooms/${id('music')}/messages`, { body });
    for (const body of ['saturday works for me', '@alice can you book the room?', 'I will bring snacks'])
      await api(bob.page, 'POST', `/api/rooms/${id(LONG_ROOM)}/messages`, { body });
    for (const n of ['bob', 'carol', 'dave', 'Bartholomew']) await inVoice(alice.page, n).waitFor();
  })();
  return housed;
}

// The comparison, drawn in a page: side by side, the diff picture and the share of pixels that differ.
async function compare(webPng, deskPng, name) {
  const { page } = await browser('compare');
  const [a, b] = await Promise.all([webPng, deskPng].map(async (p) => `data:image/png;base64,${(await readFile(p)).toString('base64')}`));
  const r = await page.evaluate(async ([a, b, tol]) => {
    const load = (src) => new Promise((ok, no) => { const i = new Image(); i.onload = () => ok(i); i.onerror = no; i.src = src; });
    const [x, y] = await Promise.all([load(a), load(b)]);
    const w = Math.max(x.width, y.width), h = Math.max(x.height, y.height);
    const px = (img) => { const c = new OffscreenCanvas(w, h); const g = c.getContext('2d'); g.drawImage(img, 0, 0); return g.getImageData(0, 0, w, h).data; };
    const [p, q] = [px(x), px(y)];
    const diff = new OffscreenCanvas(w, h);
    const dg = diff.getContext('2d');
    const d = dg.createImageData(w, h);
    let bad = 0;
    for (let i = 0; i < p.length; i += 4) {
      const off = Math.max(Math.abs(p[i] - q[i]), Math.abs(p[i + 1] - q[i + 1]), Math.abs(p[i + 2] - q[i + 2]));
      const grey = (p[i] + p[i + 1] + p[i + 2]) / 12;
      if (off > tol) { bad++; d.data.set([255, 40, 40, 255], i); } else d.data.set([grey, grey, grey, 255], i);
    }
    dg.putImageData(d, 0, 0);
    const side = new OffscreenCanvas(w * 2 + 8, h);
    const sg = side.getContext('2d');
    sg.fillStyle = '#ff00ff';
    sg.fillRect(0, 0, side.width, h);
    sg.drawImage(x, 0, 0);
    sg.drawImage(y, w + 8, 0);
    const url = async (c) => { const blob = await c.convertToBlob({ type: 'image/png' }); const s = new Uint8Array(await blob.arrayBuffer()); let t = ''; for (const v of s) t += String.fromCharCode(v); return btoa(t); };
    return { pct: (100 * bad) / (w * h), sizes: [[x.width, x.height], [y.width, y.height]], diff: await url(diff), side: await url(side) };
  }, [a, b, TOLERANCE]);
  await page.context().browser().close();
  await writeFile(`${out}${name}-diff.png`, Buffer.from(r.diff, 'base64'));
  await writeFile(`${out}${name}-side.png`, Buffer.from(r.side, 'base64'));
  const [ws, ds] = r.sizes.map((s) => s.join('x'));
  console.log(`${name}: ${r.pct.toFixed(2)} % of pixels differ (channel > ${TOLERANCE})${ws === ds ? '' : `; sizes differ: web ${ws}, desktop ${ds}`}`);
  console.log(`  ${out}${name}-side.png`);
  return r.pct;
}

// A headless sway at 1440x900, scale 1, on its own wayland socket (sway names it; an exec writes it down). Killed with the
// run, by its process group (track() in lib.mjs), never by name: other sways on the machine are not ours.
async function startSway(dir) {
  const runtime = process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid()}`; // short: wayland socket paths are limited
  const sock = join(dir, 'socket');
  await writeFile(join(dir, 'sway.conf'), [
    `output HEADLESS-1 resolution ${VIEWPORT.width}x${VIEWPORT.height} scale 1`,
    'default_border none',
    'default_floating_border none',
    `exec sh -c 'printf %s "$WAYLAND_DISPLAY" > ${sock}'`,
  ].join('\n') + '\n');
  const sway = track('sway', ['-c', join(dir, 'sway.conf')], {
    env: { ...process.env, XDG_RUNTIME_DIR: runtime, WLR_BACKENDS: 'headless', WLR_LIBINPUT_NO_DEVICES: '1', WAYLAND_DISPLAY: '', DISPLAY: '' },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  let log = '';
  sway.stderr.on('data', (b) => (log += b));
  for (let i = 0; i < 40 && !existsSync(sock); i++) await sleep(250);
  const display = existsSync(sock) ? (await readFile(sock, 'utf8')).trim() : '';
  if (!display) throw new Error(`sway did not start:\n${log}`);
  return { display, runtime };
}

// The desktop app in sway, put into `state`; resolves once it printed VESPER_DEV_READY and had time to draw.
async function shootDesktop(sway, state, path, dir) {
  const home = await mkdtemp(join(dir, 'home-'));
  if (state.settings) {
    await mkdir(join(home, '.config/Vesper'), { recursive: true });
    await writeFile(join(home, '.config/Vesper/settings.json'), JSON.stringify(state.settings));
  }
  const env = {
    ...process.env,
    HOME: home, XDG_CONFIG_HOME: join(home, '.config'), XDG_DATA_HOME: join(home, '.local/share'), XDG_CACHE_HOME: join(home, '.cache'),
    XDG_RUNTIME_DIR: sway.runtime, WAYLAND_DISPLAY: sway.display, DISPLAY: '',
    DBUS_SESSION_BUS_ADDRESS: 'disabled:', // no keyring: the throwaway home never touches the real one
    VESPER_DEV_SCENE: state.scene ?? '',
    VESPER_DEV_CLOCK: '19:41', // the web scenes' 7:41 PM (shotlib.mjs)
    LANG: 'en_US.UTF-8', LC_ALL: 'en_US.UTF-8', LANGUAGE: 'en_US', // Playwright's Chromium is en-US
    ...state.env,
    ...(state.user && { VESPER_DEV_SERVER: ORIGIN, VESPER_DEV_USER: state.user, VESPER_DEV_PASSWORD: PASSWORD }),
  };
  const child = track(app, [], { env, stdio: ['ignore', 'ignore', 'pipe'] });
  let log = '';
  const ready = new Promise((ok, no) => {
    child.stderr.on('data', (b) => { log += b; if (log.includes('VESPER_DEV_READY')) ok(); });
    child.exited.then(() => no(new Error(`the desktop app exited:\n${log.slice(-2000)}`)));
  });
  try {
    // 180 s: past 5 sign ins a minute the server takes one a minute, and the app waits and tries again (dev.rs).
    await Promise.race([ready, sleep(180000).then(() => { throw new Error(`no VESPER_DEV_READY in 180 s:\n${log.slice(-2000)}`); })]);
    await sleep(1500); // a few frames, the logo and the fonts
    const shoot = async (to) => {
      const grim = track('grim', [to], { env: { ...process.env, XDG_RUNTIME_DIR: sway.runtime, WAYLAND_DISPLAY: sway.display }, stdio: 'inherit' });
      if ((await grim.exited) !== 0) throw new Error('grim failed');
    };
    if (state.offline) {
      // The server stops; once the app says it is offline, the picture.
      await stopServer(async () => {
        const until = performance.now() + 20000;
        while (!log.includes('VESPER_DEV_OFFLINE')) {
          if (performance.now() > until) throw new Error(`no VESPER_DEV_OFFLINE:\n${log.slice(-2000)}`);
          await sleep(20);
        }
        await sleep(1000);
        await shoot(path);
      });
      return;
    }
    if (state.frames) {
      // VESPER_DEV_SEND (or the trigger): grab frames that long after the message or the ring came.
      await state.trigger?.();
      const until = performance.now() + 15000;
      while (!log.includes('VESPER_DEV_ARRIVED')) {
        if (performance.now() > until) throw new Error(`no VESPER_DEV_ARRIVED:\n${log.slice(-2000)}`);
        await sleep(2);
      }
      const t0 = performance.now();
      for (const t of state.frames) {
        await sleep(t0 + t - performance.now());
        const at = Math.round(performance.now() - t0);
        const to = path.replace(/\.png$/, `-${t}ms.png`);
        await shoot(to);
        console.log(`  frame at ${at} ms (grim done at ${Math.round(performance.now() - t0)} ms): ${to}`);
      }
      await sleep(600);
    }
    await shoot(path);
  } finally {
    try { process.kill(-child.pid, 'SIGTERM'); } catch { /* gone */ }
    await Promise.race([child.exited, sleep(3000)]);
    try { process.kill(-child.pid, 'SIGKILL'); } catch { /* gone */ }
  }
}

await mkdir(out, { recursive: true });
const args = process.argv.slice(2);
if (args[0] === '--compare') {
  if (args.length !== 4) throw new Error('usage: node e2e/parity.mjs --compare web.png desktop.png name');
  await compare(args[1], args[2], args[3]);
  process.exit(0);
}
for (const n of args) if (!SCENES[n]) throw new Error(`unknown scene ${n}; known: ${Object.keys(SCENES).join(' ')}`);
if (!existsSync(app)) throw new Error(`no desktop app at ${app}: cd desktop && ../scripts/capped.sh cargo build`);
const run = args.length ? args : Object.keys(SCENES);

const stop = await startServer();
stopServer = (during) => stop.restart(during);
const dir = await mkdtemp(join(tmpdir(), 'vesper-parity-'));
let failed = false;
try {
  const sway = await startSway(dir);
  const ctx = await cast();
  for (const name of run) {
    const s = SCENES[name];
    const page = await s.web(ctx);
    if (page) {
      if (s.focus) { await page.mouse.move(1439, 899); await sleep(300); } else await rest(page);
      await page.screenshot({ path: `${out}${name}-web.png` });
    }
    const state = typeof s.desktop === 'function' ? await s.desktop(ctx) : s.desktop;
    await shootDesktop(sway, state, `${out}${name}-desktop.png`, dir);
    await s.after?.(ctx);
    await compare(`${out}${name}-web.png`, `${out}${name}-desktop.png`, name);
  }
} catch (e) {
  console.error(e?.stack ?? e);
  failed = true;
} finally {
  await stop(); // also ends sway and anything else track() started
  await rm(dir, { recursive: true, force: true });
}
process.exit(failed ? 1 : 0);
