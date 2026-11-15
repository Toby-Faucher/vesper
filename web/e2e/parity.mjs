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
import { LONG_ROOM, VIEWPORT, api, cast, home, rest, shotBrowser } from './shotlib.mjs';

const out = fileURLToPath(new URL('parity/', import.meta.url));
const app = process.env.VESPER_DESKTOP_BIN || fileURLToPath(new URL('../../desktop/target/debug/vesper-desktop', import.meta.url));
const TOLERANCE = 24;
const PASSWORD = 'test-password-1'; // what signup() in lib.mjs gives every account

// web(ctx) returns the page to shoot; desktop is the VESPER_DEV_* state: `user` signs in, `scene` is sign-in, room:<name>
// or settings (src/ui/dev.rs), and `settings` is written to the app's settings.json first.
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
  timeline: {
    web: async ({ alice, seed }) => {
      await seed();
      await home(alice.page);
      await openRoom(alice.page, 'general');
      return alice.page;
    },
    desktop: { user: 'alice', scene: 'room:general' },
  },
  shell: {
    web: async (ctx) => { await house(ctx); await openRoom(ctx.alice.page, 'general'); return ctx.alice.page; },
    desktop: { user: 'alice', scene: 'room:general' },
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
  },
};

// shots.mjs's full house without alice in the call (the call bar is its own scene): bob, carol (muted), dave (deafened)
// and Bartholomew in voice in music, unread counts on music and a mention in the long room. The browsers stay open.
let housed;
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
    ...(state.user && { VESPER_DEV_SERVER: ORIGIN, VESPER_DEV_USER: state.user, VESPER_DEV_PASSWORD: PASSWORD }),
  };
  const child = track(app, [], { env, stdio: ['ignore', 'ignore', 'pipe'] });
  let log = '';
  const ready = new Promise((ok, no) => {
    child.stderr.on('data', (b) => { log += b; if (log.includes('VESPER_DEV_READY')) ok(); });
    child.exited.then(() => no(new Error(`the desktop app exited:\n${log.slice(-2000)}`)));
  });
  try {
    await Promise.race([ready, sleep(60000).then(() => { throw new Error(`no VESPER_DEV_READY in 60 s:\n${log.slice(-2000)}`); })]);
    await sleep(1500); // a few frames, the logo and the fonts
    const grim = track('grim', [path], { env: { ...process.env, XDG_RUNTIME_DIR: sway.runtime, WAYLAND_DISPLAY: sway.display }, stdio: 'inherit' });
    if ((await grim.exited) !== 0) throw new Error('grim failed');
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
const dir = await mkdtemp(join(tmpdir(), 'vesper-parity-'));
let failed = false;
try {
  const sway = await startSway(dir);
  const ctx = await cast();
  for (const name of run) {
    const s = SCENES[name];
    const page = await s.web(ctx);
    await rest(page);
    await page.screenshot({ path: `${out}${name}-web.png` });
    await shootDesktop(sway, s.desktop, `${out}${name}-desktop.png`, dir);
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
