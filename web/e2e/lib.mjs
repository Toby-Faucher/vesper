// Helpers for the e2e tests: a release server on a throwaway data dir, its admin CLI, and browsers with fake microphones.
import { spawn, execFile } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { chromium } from 'playwright-core';

const web = fileURLToPath(new URL('..', import.meta.url));
const bin = process.env.VESPER_SERVER_BIN || '../target/release/vesper-server';
export const ORIGIN = 'http://127.0.0.1:18090';
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let env = null;
const browsers = [];

// Starts the server on 127.0.0.1:18090 (UDP 18091) with a new data dir and waits for /health. Returns stop().
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
  const server = spawn(bin, [], { cwd: web, stdio: ['ignore', 'inherit', 'inherit'], env });
  let spawnError = null;
  server.once('error', (e) => (spawnError = e)); // ENOENT etc.: without a listener this would crash the process
  const exited = new Promise((r) => server.once('exit', r).once('error', r));
  const stop = async () => {
    process.off('SIGINT', onSigint);
    await Promise.all(browsers.map((b) => b.close()));
    if (server.exitCode === null && !spawnError) server.kill('SIGTERM');
    await exited;
    await rm(dataDir, { recursive: true, force: true });
  };
  const onSigint = () => stop().finally(() => process.exit(130));
  process.on('SIGINT', onSigint);
  let up = false;
  for (let i = 0; i < 40 && !up && server.exitCode === null && !spawnError; i++) {
    up = await fetch(ORIGIN + '/health').then((r) => r.ok, () => false);
    if (!up) await sleep(250);
  }
  if (!up) {
    await stop();
    throw new Error(spawnError ? `cannot start ${bin}: ${spawnError.message}` : 'server did not answer /health');
  }
  return stop;
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

// Opens an invite link and creates the account `name` (display name `name` too). Waits for the voice screen.
export async function signup(page, link, name) {
  await page.goto(link);
  await page.getByLabel('Username').fill(name);
  await page.getByLabel('Display name').fill(name);
  await page.getByLabel('Password').fill('test-password-1');
  await page.getByRole('button', { name: 'Create account' }).click();
  await page.getByText(`Signed in as`).waitFor();
}

// The securitypolicyviolation events the page saw since it loaded; must be [].
export const cspViolations = (page) => page.evaluate(() => window.__csp);
