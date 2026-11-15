// What shots.mjs and parity.mjs share: the 1440x900 browsers with a fixed clock, the API from a page, the cast
// (alice the owner and bob the member, signed up, with seed() for content) and the rooms of the timeline and upload
// scenes (plans, photos). Needs startServer() from lib.mjs first.
import { crc32, deflateSync } from 'node:zlib';
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

// The timeline: a room "plans" with three days of talk (the harness moves the older messages back a day or two, since
// the server stamps them now), groups, a code block, a reply, an edited, a pinned and a deleted message, reactions, a
// long message, a mention of alice and a New row (alice read up to the deleted one). alice opens it at the New row.
let planned;
export function plans({ alice, bob }) {
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
    m.push(await post('a', 'this is the config I use for voice:\n```json\n{ "bitrate": 64, "noise": "deepfilter", "ptt": false }\n```\nthe `deepfilter` mode helps a lot'));
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
export async function backdate(page, shift, { limit, holdOlder } = {}) {
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
// Uploads: a room "photos" with a wide and a tall image and a file card. `png(w, h)` draws a soft two-colour gradient
// (no binary in the repo). The upload scenes swap XMLHttpRequest for a fake that holds each upload at a set point.
export function png(w, h, [r0, g0, b0], [r1, g1, b1]) {
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
export function photosRoom({ alice, bob }) {
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
