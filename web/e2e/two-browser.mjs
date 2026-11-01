// Two-browser voice test. Starts the release server on a throwaway data dir, makes the owner invite with the CLI, signs
// alice up, has her invite bob through the API, and joins both into `general` with fake microphones. Checks that audio
// flows both ways, that bob hears alice, and that each sees the other in the people list. Stops the server.
// Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { browser, cli, signup, sleep, startServer } from './lib.mjs';

const speech = fileURLToPath(new URL('fixtures/speech.wav', import.meta.url));

async function join(page) {
  await page.getByLabel('Room').selectOption({ label: 'general' });
  await page.getByRole('button', { name: 'Join' }).click();
  await page.getByRole('button', { name: 'Leave' }).waitFor();
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

  assert.deepEqual([...alice.errors, ...bob.errors], [], 'page errors');
  console.log('e2e two-browser: PASS');
} finally {
  await stop();
}
