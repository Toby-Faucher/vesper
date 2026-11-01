// Screenshot scenes for review: node e2e/shots.mjs [names...] (npm run shots -- login room). All scenes when none are named.
// Starts the release server like the e2e tests (needs ../target/release/vesper-server and dist/), signs up the owner alice
// and the member bob, seeds three rooms and 40 messages, then writes e2e/shots/<name>.png at 1440x900, full page.
// The `tokens` scene shoots /_gallery from a Vite dev server on a spare port, since the build leaves the gallery out.
import { spawn } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import { ORIGIN, browser, cli, signup, sleep, startServer } from './lib.mjs';

const VIEWPORT = { width: 1440, height: 900 };
const out = fileURLToPath(new URL('shots/', import.meta.url));
const speech = fileURLToPath(new URL('fixtures/speech.wav', import.meta.url));

const scenes = new Map();
// fn({ alice, bob, api, open }) returns the page to shoot. alice and bob are signed-in { page, errors }; api(page, method, path,
// body) calls the API from that page (cookie and Origin as the app sends them); open(url) is a fresh signed-out page.
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

scene('tokens', async ({ open }) => {
  const port = await new Promise((r) => { const s = createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)); }); });
  const web = fileURLToPath(new URL('..', import.meta.url));
  const vite = spawn('node_modules/.bin/vite', ['--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: web, stdio: 'ignore' });
  stops.push(() => vite.kill('SIGTERM'));
  const url = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 80 && !(await fetch(url).then((r) => r.ok, () => false)); i++) await sleep(250);
  const page = await open(`${url}/_gallery`);
  await page.getByRole('heading', { name: 'Tokens' }).waitFor();
  return page;
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

  for (const name of run) {
    const page = await scenes.get(name)({ alice, bob, api, open });
    const path = `${out}${name}.png`;
    await page.screenshot({ path, fullPage: true });
    console.log(path);
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
