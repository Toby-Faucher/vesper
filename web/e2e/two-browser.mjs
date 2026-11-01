// Two-browser voice and chat test. Starts the release server on a throwaway data dir, makes the owner invite with the CLI, signs
// alice up, has her invite bob through the API, and joins both into `general` with fake microphones. Checks that audio
// flows both ways (bob to alice only as rising packets), that bob hears alice, and that each sees the other in the people list.
// Then chat: Markdown and raw HTML shown safely, a reply, an edit, a PNG upload shown inline, and search that never
// returns a message from a room bob cannot see. Stops the server.
// Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { crc32, deflateSync } from 'node:zlib';
import { browser, cli, cspViolations, signup, sleep, startServer } from './lib.mjs';

const speech = fileURLToPath(new URL('fixtures/speech.wav', import.meta.url));

async function join(page) {
  await page.getByLabel('Room').selectOption({ label: 'general' });
  await page.getByRole('button', { name: 'Join' }).click();
  await page.getByRole('button', { name: 'Leave' }).waitFor();
}

// A 1x1 RGBA PNG, built here so no binary is committed.
function png1x1() {
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

// fetch from the page, so it sends the session cookie and the Origin header as the app does.
const call = (page, method, path, body) =>
  page.evaluate(
    async ([method, path, body]) => {
      const r = await fetch(path, {
        method,
        headers: body === undefined ? undefined : { 'content-type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      if (!r.ok) throw new Error(`${method} ${path}: ${r.status}`);
      return r.status === 204 ? null : r.json();
    },
    [method, path, body],
  );

const composer = (page) => page.getByPlaceholder('Message #general');
// The message whose Markdown has a bold "bob" (a reply's quoted excerpt is plain text).
const boldBob = (page) => page.locator('article').filter({ has: page.locator('strong', { hasText: 'bob' }) });

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

const stop = await startServer();
try {
  const ownerLink = await cli(['create-owner']);
  assert.match(ownerLink, /\/invite#[0-9a-f]{64}$/, 'create-owner did not print an invite link');

  const alice = await browser('alice', [`--use-file-for-fake-audio-capture=${speech}`]);
  await signup(alice.page, ownerLink, 'alice');
  // The page's fetch sends the session cookie and the Origin header, as the app does.
  const bobLink = await alice.page.evaluate(async () => {
    const r = await fetch('/api/invites', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ role: 'member', max_uses: 1, expires_hours: 1 }),
    });
    if (!r.ok) throw new Error(`POST /api/invites: ${r.status}`);
    return (await r.json()).url;
  });

  const bob = await browser('bob', []);
  await signup(bob.page, bobLink, 'bob');

  await join(alice.page);
  await sleep(2000);
  await join(bob.page);

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

  // The other direction: bob's default fake device sends a beep, so alice's packets rise too.
  const a1 = await inbound(alice.page);
  await sleep(1000);
  const a2 = await inbound(alice.page);
  console.log('alice inbound audio:', JSON.stringify(a1), '->', JSON.stringify(a2));
  assert.ok(a1?.packets > 0 && a2.packets > a1.packets, 'alice did not receive bob\'s audio');

  // presence_seen_by_both: each page lists the other in the people list.
  await alice.page.locator('li', { hasText: 'bob' }).waitFor({ timeout: 5000 });
  await bob.page.locator('li', { hasText: 'alice' }).waitFor({ timeout: 5000 });
  console.log('presence_seen_by_both: ok');

  // chat_roundtrip: Markdown renders, raw HTML stays text, the link is safe, a reply and an edit arrive live.
  await composer(alice.page).fill('hello **bob** <b>x</b> https://example.test/a');
  await composer(alice.page).press('Enter');
  const got = boldBob(bob.page);
  await got.waitFor({ timeout: 5000 });
  assert.ok((await got.innerText()).includes('<b>x</b>'), 'raw HTML was not shown as text');
  assert.equal(await got.locator('b').count(), 0, 'raw HTML became an element');
  const link = got.getByRole('link', { name: 'https://example.test/a' });
  assert.equal(await link.getAttribute('href'), 'https://example.test/a');
  assert.equal(await link.getAttribute('rel'), 'noopener noreferrer nofollow');
  assert.equal(await link.getAttribute('target'), '_blank');
  await got.getByRole('button', { name: 'Reply', exact: true }).click();
  await composer(bob.page).fill('hi alice');
  await composer(bob.page).press('Enter');
  await alice.page.locator('article', { hasText: 'Reply to alice' }).filter({ hasText: 'hi alice' }).waitFor({ timeout: 5000 });
  await boldBob(alice.page).getByRole('button', { name: 'Edit', exact: true }).click();
  await alice.page.getByLabel('Edit message').fill('hello **bob** edited');
  await alice.page.getByRole('button', { name: 'Save', exact: true }).click();
  await boldBob(bob.page).filter({ hasText: 'bob edited' }).filter({ hasText: '(edited)' }).waitFor({ timeout: 5000 });
  console.log('chat_roundtrip: ok');

  // upload_roundtrip: alice attaches a PNG and sends; bob sees it inline and it decodes as 1x1.
  await alice.page.getByLabel('Attach a file').setInputFiles({ name: 'dot.png', mimeType: 'image/png', buffer: png1x1() });
  await alice.page.getByText('Attached: dot.png').waitFor({ timeout: 5000 });
  await alice.page.getByRole('button', { name: 'Send', exact: true }).click();
  await bob.page.waitForFunction(() => document.querySelector('article img[alt="dot.png"]')?.naturalWidth === 1, null, { timeout: 5000 });
  console.log('upload_roundtrip: ok');

  // search_roundtrip: a restricted room bob is not in holds another "hello". Alice (owner) finds both, bob only his.
  const secret = await call(alice.page, 'POST', '/api/rooms', { name: 'secret' });
  await call(alice.page, 'PATCH', `/api/rooms/${secret.id}`, { restricted: true });
  await call(alice.page, 'POST', `/api/rooms/${secret.id}/messages`, { body: 'hello from the secret room', reply_to: null, attachments: [] });
  const results = (page) => page.getByLabel('Search results').locator('button', { has: page.locator('mark') });
  await alice.page.getByLabel('Search messages').fill('hello');
  await results(alice.page).nth(1).waitFor({ timeout: 5000 });
  await bob.page.getByLabel('Search messages').fill('hello');
  await results(bob.page).first().waitFor({ timeout: 5000 });
  await sleep(500); // a late second hit would show by now
  assert.equal(await results(bob.page).count(), 1, 'bob should see exactly one hit');
  assert.ok(!(await results(bob.page).first().innerText()).includes('secret'), 'bob found a message from a hidden room');
  assert.equal(await results(bob.page).locator('mark').first().innerText(), 'hello');
  const options = await bob.page.getByLabel('Room').locator('option').allInnerTexts();
  assert.ok(!options.some((o) => o.includes('secret')), 'bob sees the hidden room');
  await results(bob.page).first().click();
  await boldBob(bob.page).waitFor({ timeout: 5000 });
  console.log('search_roundtrip: ok');

  for (const p of [alice.page, bob.page]) assert.deepEqual(await cspViolations(p), [], 'CSP violations');
  console.log('no CSP violations: ok');
  assert.deepEqual([...alice.errors, ...bob.errors], [], 'page errors');
  console.log('e2e two-browser: PASS');
} finally {
  await stop();
}
