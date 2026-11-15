// What shots.mjs and parity.mjs share: the 1440x900 browsers with a fixed clock, the API from a page, and the cast
// (alice the owner and bob the member, signed up, with seed() for content). Needs startServer() from lib.mjs first.
import { fileURLToPath } from 'node:url';
import { ORIGIN, browser, cli, signedIn, signup, sleep } from './lib.mjs';

export const VIEWPORT = { width: 1440, height: 900 };
export const speech = fileURLToPath(new URL('fixtures/speech.wav', import.meta.url));
// Every clock time in the pictures reads 7:41 PM, so two runs give the same pixels (the seeded messages carry the real
// time). Dates and relative times ("in 7 days") are left alone.
const fixedTimes = () => {
  const Real = Intl.DateTimeFormat;
  const at = new Date(2026, 10, 1, 19, 41);
  function Fixed(locale, o = {}) {
    const f = new Real(locale, o);
    if (!('hour' in o) || 'day' in o || 'dateStyle' in o) return f;
    return Object.create(f, { format: { value: () => f.format(at) }, formatToParts: { value: () => f.formatToParts(at) } }); // format is a getter: define, not assign
  }
  Fixed.prototype = Real.prototype;
  Fixed.supportedLocalesOf = Real.supportedLocalesOf;
  Intl.DateTimeFormat = Fixed;
};
export const shotBrowser = async (...args) => {
  const b = await browser(...args);
  await b.page.addInitScript(fixedTimes);
  return b;
};

export async function api(page, method, path, body) {
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

export async function home(page) {
  await page.goto(`${ORIGIN}/`);
  await signedIn(page);
  await page.waitForURL(/\/r\/\d+$/);
}
// Shows the page with nothing focused and the pointer out of the way, so no hover or focus ring is in the picture.
export async function rest(page) {
  await page.evaluate(() => document.activeElement?.blur());
  await page.mouse.move(1439, 899);
  await sleep(300);
}

export const LONG_ROOM = 'weekend plans and the games list'; // 32 characters, the server's limit
export const LINES = [
  'morning all', 'anyone up for a game tonight?', 'I can do 9', 'same, after dinner', 'what are we playing',
  'the co-op one again?', 'sure', 'I fixed my mic, tell me if it still crackles', 'sounds fine to me', 'link to the patch notes: https://example.com/notes',
];

// Signs up alice (the owner, whose fake mic plays speech) and bob. `open(url)` is a fresh signed-out page;
// `seed()` (once) makes two more rooms and 40 messages in general.
export async function cast() {
  const fresh = [];
  const open = async (url) => {
    const b = await shotBrowser(`fresh${fresh.length}`, [], { viewport: VIEWPORT });
    fresh.push(b);
    await b.page.goto(url);
    return b.page;
  };
  const alice = await shotBrowser('alice', [`--use-file-for-fake-audio-capture=${speech}`], { viewport: VIEWPORT });
  await signup(alice.page, await cli(['create-owner']), 'alice');
  const bobLink = (await api(alice.page, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 })).url;
  const bob = await shotBrowser('bob', [], { viewport: VIEWPORT });
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
  return { alice, bob, open, seed, fresh };
}
