// Two-browser screen share test. Alice and bob join `general`; alice shares a fake 1280x720 screen. Checks that bob gets
// video, that the SFU's transport-cc feedback lets alice's bandwidth estimate pass 2 Mbps, that hiding the tile stops the
// encoder (0 watching, no bytes sent), that showing it again resumes, that Stop removes bob's panel, and that a DM call has
// no "Share screen" button.
// Needs ../target/release/vesper-server and dist/ (npm run build). Run: node e2e/share.mjs
import assert from 'node:assert/strict';
import { ORIGIN, browser, cli, openRoom, signup, sleep, startServer } from './lib.mjs';

// A 1280x720 canvas that redraws every 33 ms, so the encoder always has new content.
const fakeScreen = () => {
  navigator.mediaDevices.getDisplayMedia = async () => {
    const c = document.createElement('canvas');
    c.width = 1280;
    c.height = 720;
    const g = c.getContext('2d');
    let n = 0;
    setInterval(() => {
      n++;
      g.fillStyle = `hsl(${(n * 7) % 360} 60% 40%)`;
      g.fillRect(0, 0, 1280, 720);
      g.fillStyle = '#fff';
      g.font = '120px monospace';
      g.fillText(String(n), 100 + (n % 600), 300 + (n % 200));
      for (let i = 0; i < 200; i++) {
        g.fillStyle = `rgb(${(i * 37 + n * 11) % 256} ${(i * 91 + n * 5) % 256} ${(i * 53 + n * 3) % 256})`;
        g.fillRect((i * 97 + n * 13) % 1240, (i * 53 + n * 7) % 680, 40, 40);
      }
    }, 33);
    return c.captureStream(30);
  };
};

const join = async (page) => {
  await openRoom(page, 'general');
  await page.getByRole('button', { name: 'Join' }).click();
  await page.getByRole('button', { name: 'Leave' }).waitFor();
};

// Polls `fn` until it returns truthy; fails with `what` after `ms`. Returns the elapsed ms.
async function until(what, ms, fn) {
  const t0 = Date.now();
  for (;;) {
    if (await fn()) return Date.now() - t0;
    assert.ok(Date.now() - t0 < ms, `${what}: not within ${ms} ms`);
    await sleep(250);
  }
}

// The first stats record of `type` (and video kind where it has one) over the open connections.
const stat = (page, type, pick) =>
  page.evaluate(async ([type, pick]) => {
    for (const pc of window.__pcs.filter((p) => p.connectionState !== 'closed')) {
      for (const r of (await pc.getStats()).values()) {
        if (r.type !== type || (r.kind && r.kind !== 'video')) continue;
        if (type === 'candidate-pair' && !(r.nominated || r.state === 'succeeded')) continue;
        if (r[pick] !== undefined) return r[pick];
      }
    }
    return null;
  }, [type, pick]);
const outVideo = (page) =>
  page.evaluate(async () => {
    for (const pc of window.__pcs.filter((p) => p.connectionState !== 'closed'))
      for (const r of (await pc.getStats()).values())
        if (r.type === 'outbound-rtp' && r.kind === 'video')
          return { bytes: r.bytesSent, enc: r.framesEncoded, sent: r.framesSent, rtx: r.retransmittedBytesSent ?? 0 };
    return null;
  });
const bytesSent = (page) => stat(page, 'outbound-rtp', 'bytesSent');
const bytesReceived = (page) => stat(page, 'inbound-rtp', 'bytesReceived');
const tileVideoWidth = (page) => page.evaluate(() => document.querySelector('[aria-label="alice\'s screen"] video')?.videoWidth ?? 0);

const stop = await startServer();
try {
  const ownerLink = await cli(['create-owner']);
  const alice = await browser('alice');
  const bob = await browser('bob');
  for (const p of [alice.page, bob.page]) await p.addInitScript(fakeScreen);
  await signup(alice.page, ownerLink, 'alice');
  const bobLink = await alice.page.evaluate(async () => {
    const r = await fetch('/api/invites', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ role: 'member', max_uses: 1, expires_hours: 1 }),
    });
    if (!r.ok) throw new Error(`POST /api/invites: ${r.status}`);
    return (await r.json()).url;
  });
  await signup(bob.page, bobLink, 'bob');
  await join(alice.page);
  await join(bob.page);

  await alice.page.getByRole('button', { name: 'Share screen' }).click();
  await alice.page.getByRole('dialog', { name: 'Share your screen' }).waitFor();
  await alice.page.getByRole('button', { name: 'Start sharing' }).click();

  // Bob gets video, and bytes keep arriving.
  const tile = bob.page.getByLabel("alice's screen");
  let ms = await until('bob tile video', 10000, async () => (await tileVideoWidth(bob.page)) > 0);
  console.log(`tile video: ${ms} ms, ${await tileVideoWidth(bob.page)} px wide`);
  const rx0 = await bytesReceived(bob.page);
  await sleep(2000);
  const rx1 = await bytesReceived(bob.page);
  assert.ok(rx1 > rx0, `bob's inbound video bytes did not grow (${rx0} -> ${rx1})`);

  // Alice's card counts bob, and the SFU's transport-cc feedback lets her estimate pass 2 Mbps.
  await alice.page.getByText('1 watching').waitFor({ timeout: 5000 });
  let peak = 0;
  ms = await until('availableOutgoingBitrate > 2 Mbps', 20000, async () => {
    peak = Math.max(peak, (await stat(alice.page, 'candidate-pair', 'availableOutgoingBitrate')) ?? 0);
    return peak > 2_000_000;
  });
  console.log(`availableOutgoingBitrate passed 2 Mbps after ${ms} ms (peak ${peak})`);

  // Watched baseline: alice's video payload rate over 3 s.
  const w0 = await outVideo(alice.page);
  await sleep(3000);
  const w1 = await outVideo(alice.page);
  const watchedRate = (w1.bytes - w0.bytes) / 3;
  console.log(`watched: ${Math.round(watchedRate)} B/s, ${(w1.enc - w0.enc) / 3} frames/s encoded`);
  assert.ok(watchedRate > 50_000, `alice sends too little while watched (${watchedRate} B/s)`);

  // Bob hides the tile: after the 10 s grace the server counts 0 and the encoder goes quiet.
  await tile.getByRole('button', { name: 'Hide' }).click();
  await bob.page.getByText("alice's screen is hidden.").waitFor();
  ms = await until('0 watching', 15000, async () => alice.page.getByText('0 watching').isVisible());
  console.log(`0 watching after ${ms} ms`);
  // Per-second deltas for 14 s. After a 2 s settle the encoder must be still and bytes under 2% of the watched rate.
  let prev = await outVideo(alice.page);
  let bursts = 0;
  const base = prev;
  for (let i = 1; i <= 14; i++) {
    await sleep(1000);
    const cur = await outVideo(alice.page);
    const d = { bytes: cur.bytes - prev.bytes, enc: cur.enc - prev.enc, sent: cur.sent - prev.sent, rtx: cur.rtx - prev.rtx };
    console.log(`unwatched s${i}: bytes ${d.bytes} framesEncoded ${d.enc} framesSent ${d.sent} rtxBytes ${d.rtx}`);
    if (i > 2) {
      if (d.bytes) bursts++;
      assert.equal(d.enc, 0, `encoder still running with no watcher (s${i}: ${d.enc} frames)`);
    }
    prev = cur;
  }
  // Bytes are not zero: about 10 kB leaves every ~5 s with no frames encoded. Bound the mean after the settle, not each second.
  const mean = (prev.bytes - base.bytes) / 14;
  console.log(`unwatched mean ${Math.round(mean)} B/s (${((100 * mean) / watchedRate).toFixed(2)}% of watched), ${bursts} bursts`);
  assert.ok(mean < watchedRate * 0.02, `alice still sends ${Math.round(mean)} B/s with no watcher (watched ${Math.round(watchedRate)} B/s)`);

  // Bob shows it again and video resumes.
  await bob.page.getByRole('button', { name: 'Show' }).click();
  const rx2 = await bytesReceived(bob.page);
  ms = await until('video resumes', 5000, async () => (await bytesReceived(bob.page)) > rx2 + 5000 && (await tileVideoWidth(bob.page)) > 0);
  console.log(`video resumed after ${ms} ms`);
  await alice.page.getByText('1 watching').waitFor({ timeout: 5000 });

  // Alice stops; bob's panel goes away.
  await alice.page.getByRole('button', { name: 'Stop', exact: true }).click();
  await bob.page.getByLabel("alice's screen").waitFor({ state: 'detached', timeout: 5000 });
  await bob.page.getByText('1 screen', { exact: true }).waitFor({ state: 'detached', timeout: 5000 });

  // A DM call has no share button.
  const aliceId = await alice.page.evaluate(async () => (await (await fetch('/api/me')).json()).user.user_id);
  const dm = await bob.page.evaluate(async (id) => {
    const r = await fetch('/api/dms', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ user_id: id }) });
    if (!r.ok) throw new Error(`POST /api/dms: ${r.status}`);
    return r.json();
  }, aliceId);
  await bob.page.getByRole('button', { name: 'Leave' }).click();
  await bob.page.goto(`${ORIGIN}/r/${dm.id}`);
  await bob.page.getByRole('button', { name: 'Join' }).click();
  await bob.page.getByRole('button', { name: 'Leave' }).waitFor();
  await bob.page.getByRole('button', { name: 'Mute' }).or(bob.page.getByRole('button', { name: 'Unmute' })).first().waitFor();
  assert.equal(await bob.page.getByRole('button', { name: 'Share screen' }).count(), 0, 'a DM call shows "Share screen"');
  console.log('dm_no_share: ok');

  for (const [n, e] of [['alice', alice.errors], ['bob', bob.errors]]) assert.deepEqual(e, [], `${n} page errors`);
  console.log('share: ok');
} finally {
  await stop();
}
