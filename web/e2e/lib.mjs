// Helpers for the e2e tests: a release server on a throwaway data dir, its admin CLI, and browsers with fake microphones.
import { spawn, execFile } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { crc32, deflateSync } from 'node:zlib';
import { chromium } from 'playwright-core';

const web = fileURLToPath(new URL('..', import.meta.url));
const bin = process.env.VESPER_SERVER_BIN || '../target/release/vesper-server';
export const ORIGIN = 'http://127.0.0.1:18090';
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let env = null;
const browsers = [];
let dev = null; // { url: Promise<string>, kill() }

// Starts the server on 127.0.0.1:18090 (UDP 18091) with a new data dir and waits for /health. Returns stop(), which also
// has stop.restart(during, extraEnv): it stops the server, awaits during() while it is down, then starts it again on the same
// data, with extraEnv added to its environment.
export async function startServer() {
  const dataDir = await mkdtemp(join(tmpdir(), 'vesper-e2e-'));
  env = {
    ...process.env,
    VESPER_DATA_DIR: dataDir,
    VESPER_ORIGIN: ORIGIN,
    VESPER_HTTP_BIND: '127.0.0.1:18090',
    VESPER_UDP_BIND: '127.0.0.1:18091',
    VESPER_WEB_DIR: 'dist',
  };
  let server, spawnError, exited;
  const launch = () => {
    server = spawn(bin, [], { cwd: web, stdio: ['ignore', 'inherit', 'inherit'], env });
    spawnError = null;
    server.once('error', (e) => (spawnError = e)); // ENOENT etc.: without a listener this would crash the process
    exited = new Promise((r) => server.once('exit', r).once('error', r));
  };
  const kill = async () => {
    if (server.exitCode === null && !spawnError) server.kill('SIGTERM');
    await exited;
  };
  const stop = async () => {
    process.off('SIGINT', onSigint);
    await Promise.all(browsers.map((b) => b.close()));
    dev?.kill();
    dev = null;
    await kill();
    await rm(dataDir, { recursive: true, force: true });
  };
  const onSigint = () => stop().finally(() => process.exit(130));
  process.on('SIGINT', onSigint);
  const healthy = async () => {
    let up = false;
    for (let i = 0; i < 40 && !up && server.exitCode === null && !spawnError; i++) {
      up = await fetch(ORIGIN + '/health').then((r) => r.ok, () => false);
      if (!up) await sleep(250);
    }
    if (!up) {
      await stop();
      throw new Error(spawnError ? `cannot start ${bin}: ${spawnError.message}` : 'server did not answer /health');
    }
  };
  launch();
  await healthy();
  stop.restart = async (during, extraEnv = {}) => {
    await kill();
    await during();
    Object.assign(env, extraEnv); // kept for later starts; '' unsets a VESPER_ variable
    launch();
    await healthy();
  };
  return stop;
}

// A Vite dev server for the dev-only /_gallery pages (the build leaves them out), started once on a spare port; the stop()
// of startServer kills it. Returns its URL.
export function devServer() {
  if (!dev) {
    let vite;
    const url = (async () => {
      const port = await new Promise((r) => { const s = createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)); }); });
      vite = spawn('node_modules/.bin/vite', ['--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: web, stdio: 'ignore' });
      const u = `http://127.0.0.1:${port}`;
      for (let i = 0; i < 80 && !(await fetch(u).then((r) => r.ok, () => false)); i++) await sleep(250);
      return u;
    })();
    dev = { url, kill: () => vite?.kill('SIGTERM') };
  }
  return dev.url;
}

// Runs `vesper-server <args>` on the same data dir and returns its stdout, trimmed.
export async function cli(args) {
  const { stdout } = await promisify(execFile)(bin, args, { cwd: web, env, timeout: 30000 });
  return stdout.trim();
}

// Records every RTCPeerConnection so the test can read its stats.
const hook = () => {
  const Orig = window.RTCPeerConnection;
  window.__pcs = [];
  window.RTCPeerConnection = function (...a) {
    const pc = new Orig(...a);
    window.__pcs.push(pc);
    return pc;
  };
  window.RTCPeerConnection.prototype = Orig.prototype;
};

// A headless Chromium with a fake microphone; `args` are extra Chromium flags, `viewport` e.g. { width: 1440, height: 900 }.
// Returns { page, errors }.
export async function browser(name, args = [], { viewport } = {}) {
  const b = await chromium.launch({
    executablePath: process.env.E2E_CHROMIUM || undefined, // local only: a browser Playwright did not download
    args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream', '--autoplay-policy=no-user-gesture-required', ...args],
  });
  browsers.push(b);
  const page = await (await b.newContext({ permissions: ['microphone'], ...(viewport && { viewport }) })).newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(`${name}: ${e}`));
  // The server sends a strict CSP: a blocked load in the page shows as a console error and an event. A block inside a worker
  // shows neither; only the optional denoiser step in flows.mjs catches that, and CI skips it.
  page.on('console', (m) => /content security policy/i.test(m.text()) && errors.push(`${name}: ${m.text()}`));
  await page.addInitScript(hook);
  await page.addInitScript(() => {
    window.__csp = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__csp.push(`${e.effectiveDirective} ${e.blockedURI}`));
  });
  // CI does not build the wasm denoiser, so use browser suppression instead of waiting for the fallback.
  await page.addInitScript(() => localStorage.setItem('vesper.audio', JSON.stringify({ mode: 'browser' })));
  return { page, errors };
}

// Opens an invite link and creates the account `name` (display name `display`, default `name`), skips the welcome steps
// and waits for the app shell.
export async function signup(page, link, name, display = name) {
  await page.goto(link);
  await page.getByLabel('Username').fill(name);
  await page.getByLabel('Display name').fill(display);
  await page.getByLabel('Password', { exact: true }).fill('test-password-1');
  await page.getByRole('button', { name: 'Create account' }).click();
  await skipWelcome(page);
}

// On /welcome (after a signup): presses "Skip for now" and waits for the app shell.
export async function skipWelcome(page) {
  await page.waitForURL(/\/welcome$/);
  await page.getByRole('button', { name: 'Skip for now' }).click();
  await signedIn(page);
}

// Waits for the signed-in shell: the Rooms navigation in the sidebar.
export const signedIn = (page) => page.getByRole('navigation', { name: 'Rooms' }).waitFor();

// The sidebar link of a room. Its accessible name starts with the room name ("general, 3 unread").
export const roomLink = (page, name) =>
  page.getByRole('navigation', { name: 'Rooms' }).getByRole('link', { name: new RegExp('^' + name + '\\b') });

// Opens a room from the sidebar and waits for its address.
export async function openRoom(page, name) {
  await roomLink(page, name).click();
  await page.waitForURL(/\/r\/\d+$/);
  await page.getByRole('heading', { level: 1, name, exact: true }).waitFor();
}

// The id of the open room, from /r/<id>.
export const roomId = (page) => Number(new URL(page.url()).pathname.match(/^\/r\/(\d+)/)?.[1]);

// A person in voice in the sidebar (a PresenceList item; the room row around it is an li too).
export const inVoice = (page, name) => page.locator('li[data-speaking]', { hasText: name });

// Picks an item ("Edit", "Copy text", "Delete") from a message's More menu. The toolbar shows on hover; the menu is a
// popover outside the article, so the item is found on the page.
export async function messageMenu(article, item) {
  await article.getByRole('button', { name: 'More', exact: true }).click();
  await article.page().getByRole('menuitem', { name: item, exact: true }).click();
}

// The message box of the open room ("Message <room>" is its placeholder; the edit box is "Edit message").
export const composer = (page) => page.getByRole('textbox', { name: 'Message', exact: true });

// Opens the Account menu at the bottom of the sidebar and signs out.
export async function logout(page) {
  await page.getByRole('button', { name: 'Account' }).click();
  await page.getByRole('menuitem', { name: 'Log out' }).click();
}

// The securitypolicyviolation events the page saw since it loaded; must be [].
export const cspViolations = (page) => page.evaluate(() => window.__csp);

// Opens the search dialog (the header's Search button, or Ctrl+Shift+F with `keys`) and types `text` into it.
export async function searchFor(page, text, { keys = false } = {}) {
  if (keys) await page.keyboard.press('Control+Shift+F');
  else await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.getByLabel('Search messages').fill(text);
}

// The search dialog's result buttons that show a highlighted match.
export const searchHits = (page) => page.getByLabel('Search results').locator('button', { has: page.locator('mark') });

// A 1x1 RGBA PNG, built here so no binary is committed.
export function png1x1() {
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const out = Buffer.alloc(12 + data.length);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(crc32(body), 8 + data.length);
    return out;
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(1, 0);
  ihdr.writeUInt32BE(1, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return Buffer.concat([signature, chunk('IHDR', ihdr), chunk('IDAT', deflateSync(Buffer.from([0, 255, 0, 0, 255]))), chunk('IEND', Buffer.alloc(0))]);
}
