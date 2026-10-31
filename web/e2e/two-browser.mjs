// Two-browser voice test. Starts the release server with a throwaway key, opens two headless Chromium browsers
// with fake microphones, joins both into the room, and checks that bob receives alice's audio and hears it. Stops the server.
// Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';

const web = fileURLToPath(new URL('..', import.meta.url));
const speech = fileURLToPath(new URL('fixtures/speech.wav', import.meta.url));
const BASE = 'http://127.0.0.1:18090/';
const KEY = 'e2e-throwaway-key';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const server = spawn(process.env.VESPER_SERVER_BIN || '../target/release/vesper-server', [], {
  cwd: web,
  stdio: ['ignore', 'inherit', 'inherit'],
  env: { ...process.env, VESPER_DEV_KEY: KEY, VESPER_HTTP_BIND: '127.0.0.1:18090', VESPER_UDP_BIND: '127.0.0.1:18091', VESPER_WEB_DIR: 'dist' },
});
const exited = new Promise((r) => server.once('exit', r));
const browsers = [];

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

async function join(name, args) {
  const browser = await chromium.launch({
    executablePath: process.env.E2E_CHROMIUM || undefined, // local only: a browser Playwright did not download
    args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream', '--autoplay-policy=no-user-gesture-required', ...args],
  });
  browsers.push(browser);
  const page = await (await browser.newContext({ permissions: ['microphone'] })).newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.addInitScript(hook);
  // CI does not build the wasm denoiser, so use browser suppression instead of waiting for the fallback.
  await page.addInitScript(() => localStorage.setItem('vesper.audio', JSON.stringify({ mode: 'browser' })));
  await page.goto(BASE);
  await page.getByLabel('Name').fill(name);
  await page.getByLabel('Key').fill(KEY);
  await page.getByRole('button', { name: 'Join' }).click();
  return { page, errors };
}

const inbound = (page) =>
  page.evaluate(async () => {
    for (const pc of window.__pcs) {
      for (const r of (await pc.getStats()).values()) {
        if (r.type === 'inbound-rtp' && r.kind === 'audio')
          return { ice: pc.iceConnectionState, packets: r.packetsReceived, bytes: r.bytesReceived };
      }
    }
    return null;
  });

try {
  let up = false;
  for (let i = 0; i < 40 && !up && server.exitCode === null; i++) {
    up = await fetch(BASE + 'health').then((r) => r.ok, () => false);
    if (!up) await sleep(250);
  }
  assert.ok(up, 'server did not answer /health');

  const alice = await join('alice', [`--use-file-for-fake-audio-capture=${speech}`]);
  await sleep(2000);
  const bob = await join('bob', []);

  let first = null;
  for (let i = 0; i < 40 && !(first?.packets > 0); i++) {
    await sleep(250);
    first = await inbound(bob.page);
  }
  assert.ok(first?.packets > 0, 'bob never received audio packets');
  // inbound-rtp audioLevel stays 0 here: the app plays remote audio through Web Audio and keeps the <audio> element
  // muted, and Chrome measures that muted element. So "bob hears alice" is the speaking ring on alice's row, which
  // comes from an analyser on the received stream (Playback.speaking).
  let rings = 0;
  let last = first;
  const N = 20;
  for (let i = 0; i < N; i++) {
    await sleep(250);
    last = await inbound(bob.page);
    rings += await bob.page.locator('li.ring-green-500').count();
  }
  console.log('bob inbound audio: first', JSON.stringify(first), 'last', JSON.stringify(last), `speaking ring on ${rings}/${N} samples`);
  assert.match(last.ice, /^(connected|completed)$/, 'ICE not connected');
  assert.ok(last.packets > first.packets, 'packetsReceived did not rise');
  assert.ok(last.bytes > first.bytes, 'bytesReceived did not rise');
  assert.ok(rings > 0, 'bob never saw alice speaking: received audio is silent');
  assert.deepEqual([...alice.errors, ...bob.errors], [], 'page errors');
  console.log('e2e two-browser: PASS');
} finally {
  await Promise.all(browsers.map((b) => b.close()));
  server.kill('SIGTERM');
  await exited;
}
