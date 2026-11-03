// Web Push e2e. A fake push service (a Node HTTP server on 127.0.0.1) stands in for Google, Mozilla or Apple. In real
// Chromium: bob turns push on in Settings > Notifications (permission granted, sw.js registered), sends a test push,
// then closes his tab; alice mentions him and starts voice. The test checks every POST the server makes: a valid VAPID
// JWT for the push service's origin, an RFC 8291 body that decrypts with bob's key, and no message text or name in the
// clear. Then the service worker's push handler is driven through the DevTools protocol.
//
// Not exercised: a real push service, the browser's own PushManager.subscribe (headless Chromium has no push service, so
// the page gets a stand-in subscription holding keys made here), the notification on screen, and notificationclick.
//
// Needs a debug server built with the test-push feature (release builds refuse it), so the server may POST to
// http://127.0.0.1: cargo build -p vesper-server --features test-push. Run: npm run e2e (or node e2e/push.mjs).
import assert from 'node:assert/strict';
import { createECDH, createHmac, createDecipheriv, createPublicKey, randomBytes, verify } from 'node:crypto';
import { existsSync } from 'node:fs';
import { createServer } from 'node:http';

process.env.VESPER_SERVER_BIN ||= '../target/debug/vesper-server';
if (!existsSync(new URL(process.env.VESPER_SERVER_BIN, new URL('..', import.meta.url)))) {
  console.error(`no ${process.env.VESPER_SERVER_BIN}: run cargo build -p vesper-server --features test-push`);
  process.exit(1);
}
process.env.VESPER_PUSH_SUBJECT = 'mailto:e2e@example.invalid';
const { ORIGIN, browser, cli, cspViolations, logout, openRoom, signup, sleep, startServer } = await import('./lib.mjs');

const b64 = (b) => Buffer.from(b).toString('base64url');

// The push service: records each POST and answers 201.
const got = [];
const fake = createServer((req, res) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    got.push({ url: req.url, headers: req.headers, body: Buffer.concat(chunks) });
    res.writeHead(201).end();
  });
});
await new Promise((r) => fake.listen(0, '127.0.0.1', r));
const fakeOrigin = `http://127.0.0.1:${fake.address().port}`;

async function nextPush(what) {
  for (let i = 0; i < 40 && !got.length; i++) await sleep(250);
  assert.ok(got.length, `no push for ${what}`);
  return got.shift();
}

// bob's browser keys: the private half stays here.
const ua = createECDH('prime256v1');
ua.generateKeys();
const auth = randomBytes(16);

// RFC 8291 decryption, apart from the server's code.
function decrypt(body) {
  const salt = body.subarray(0, 16);
  assert.equal(body.readUInt32BE(16), 4096);
  assert.equal(body[20], 65);
  const asPublic = body.subarray(21, 86);
  const hmac = (key, data) => createHmac('sha256', key).update(data).digest();
  const ecdh = ua.computeSecret(asPublic);
  const ikm = hmac(hmac(auth, ecdh), Buffer.concat([Buffer.from('WebPush: info\0'), ua.getPublicKey(), asPublic, Buffer.from([1])]));
  const prk = hmac(salt, ikm);
  const cek = hmac(prk, Buffer.from('Content-Encoding: aes128gcm\0\x01')).subarray(0, 16);
  const nonce = hmac(prk, Buffer.from('Content-Encoding: nonce\0\x01')).subarray(0, 12);
  const data = body.subarray(86);
  const d = createDecipheriv('aes-128-gcm', cek, nonce);
  d.setAuthTag(data.subarray(data.length - 16));
  const plain = Buffer.concat([d.update(data.subarray(0, data.length - 16)), d.final()]);
  assert.equal(plain.at(-1), 2, 'last-record delimiter');
  return JSON.parse(plain.subarray(0, -1).toString());
}

// The Authorization header: an ES256 JWT for the push service's origin, signed by the key the browser was given.
function checkVapid(headers, serverKey) {
  const [, jwt, k] = headers.authorization.match(/^vapid t=([^,]+), k=(.+)$/);
  assert.equal(k, serverKey, 'k is the key the browser subscribed with');
  const [h, c, s] = jwt.split('.');
  const point = Buffer.from(k, 'base64url');
  const key = createPublicKey({ format: 'jwk', key: { kty: 'EC', crv: 'P-256', x: b64(point.subarray(1, 33)), y: b64(point.subarray(33)) } });
  assert.ok(verify('sha256', Buffer.from(`${h}.${c}`), { key, dsaEncoding: 'ieee-p1363' }, Buffer.from(s, 'base64url')), 'VAPID signature');
  const claims = JSON.parse(Buffer.from(c, 'base64url'));
  assert.equal(claims.aud, fakeOrigin);
  assert.equal(claims.sub, 'mailto:e2e@example.invalid');
  assert.ok(claims.exp > Date.now() / 1000 && claims.exp <= Date.now() / 1000 + 24 * 3600, 'exp within 24 h');
}

// Headless Chromium has no push service: the page's PushManager hands out a subscription with bob's keys instead.
const fakeSubscription = ([endpoint, p256dh, auth]) => {
  const sub = {
    endpoint,
    options: { applicationServerKey: null },
    toJSON: () => ({ endpoint, keys: { p256dh, auth } }),
    unsubscribe: async () => (localStorage.removeItem('fake-sub'), true),
  };
  PushManager.prototype.subscribe = async function (o) {
    sub.options.applicationServerKey = new Uint8Array(o.applicationServerKey).buffer;
    localStorage.setItem('fake-sub', '1');
    return sub;
  };
  PushManager.prototype.getSubscription = async function () {
    return localStorage.getItem('fake-sub') ? sub : null;
  };
};

const stop = await startServer();
try {
  const alice = await browser('alice');
  const a = alice.page;
  await signup(a, await cli(['create-owner']), 'alice');
  const bob = await browser('bob', [], { fullChromium: true });
  const b = bob.page;
  await b.context().grantPermissions(['notifications'], { origin: ORIGIN });
  await b.addInitScript(fakeSubscription, [`${fakeOrigin}/push/bob`, b64(ua.getPublicKey()), b64(auth)]);
  await signup(b, await cli(['invite']), 'bob');

  // Settings > Notifications: off until bob turns it on; then the worker is registered and the server has the device.
  await b.goto(`${ORIGIN}/settings/notifications`);
  const toggle = b.getByRole('switch', { name: 'Notify this device while Vesper is closed' });
  await b.getByText('Off on this device.').waitFor();
  assert.equal(await b.evaluate(() => navigator.serviceWorker.getRegistration().then((r) => !!r)), false, 'worker before the switch');
  await toggle.check();
  await b.getByText(/^On for this device/).waitFor();
  // Both kinds start off; bob turns them on.
  for (const name of ['Mentions', 'Direct messages', 'Someone starts voice']) {
    const s = b.getByRole('switch', { name, exact: true });
    assert.equal(await s.isChecked(), false, `${name} starts off`);
    await s.check();
    await sleep(300);
  }
  const kinds = await b.evaluate(() => fetch('/api/push').then((r) => r.json()).then((i) => i.devices[0]));
  assert.deepEqual([kinds.mentions, kinds.dms, kinds.voice], [true, true, true], 'switches saved');
  assert.equal(await b.evaluate(() => navigator.serviceWorker.ready.then((r) => r.active?.scriptURL)), `${ORIGIN}/sw.js`);
  await b.getByRole('list', { name: 'Devices that get notifications' }).getByText('This device').waitFor();
  const info = await b.evaluate(() => fetch('/api/push').then((r) => r.json()));
  console.log('push on in Settings, worker registered: ok');

  await b.getByRole('group', { name: 'While Vesper is closed' }).getByRole('button', { name: 'Send a test notification' }).click();
  let push = await nextPush('the test');
  assert.equal(push.url, '/push/bob');
  checkVapid(push.headers, info.public_key);
  assert.equal(push.headers['content-encoding'], 'aes128gcm');
  assert.equal(push.headers.cookie, undefined);
  assert.equal(decrypt(push.body).kind, 'test');
  console.log('test push: VAPID signed, encrypted: ok');

  // The worker shows a pushed payload as a notification, tagged by room (DevTools delivers the push).
  const cdp = await b.context().newCDPSession(b);
  const reg = new Promise((r) => cdp.on('ServiceWorker.workerRegistrationUpdated', (e) => {
    const mine = e.registrations.find((x) => x.scopeURL === `${ORIGIN}/` && !x.isDeleted);
    if (mine) r(mine.registrationId);
  }));
  await cdp.send('ServiceWorker.enable');
  const data = JSON.stringify({ kind: 'mention', room_id: 1, title: 'Vesper', body: 'alice mentioned you in general' });
  await cdp.send('ServiceWorker.deliverPushMessage', { origin: ORIGIN, registrationId: await reg, data });
  let shown = [];
  for (let i = 0; i < 20 && !shown.length; i++) {
    await sleep(250);
    shown = await b.evaluate(() => navigator.serviceWorker.ready.then((r) => r.getNotifications())
      .then((ns) => ns.map((n) => ({ title: n.title, body: n.body, tag: n.tag }))));
  }
  assert.deepEqual(shown, [{ title: 'Vesper', body: 'alice mentioned you in general', tag: 'mention-1' }]);
  console.log('worker shows the push as a notification: ok');
  assert.deepEqual(await cspViolations(b), [], 'CSP violations');

  // bob closes Vesper: no socket, so a mention and the start of voice reach him by push, without the message text.
  await b.close();
  await sleep(500);
  await openRoom(a, 'general');
  const general = Number(new URL(a.url()).pathname.split('/')[2]);
  const r = await a.evaluate((id) => fetch(`/api/rooms/${id}/messages`, {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ body: '@bob the secret plan' }),
  }).then((r) => r.status), general);
  assert.equal(r, 201);
  push = await nextPush('the mention');
  checkVapid(push.headers, info.public_key);
  for (const clear of ['secret', 'alice', 'general']) assert.ok(!push.body.includes(clear), `${clear} in the clear`);
  assert.deepEqual(decrypt(push.body), { kind: 'mention', room_id: general, title: 'Vesper', body: 'alice mentioned you in general', icon: null });
  console.log('mention push: no message text: ok');

  await a.getByRole('button', { name: 'Join' }).click();
  await a.getByRole('button', { name: 'Leave' }).waitFor();
  push = await nextPush('voice');
  assert.deepEqual(decrypt(push.body), { kind: 'voice', room_id: general, title: 'Vesper', body: 'alice started voice in general', icon: null });
  assert.equal(push.headers.ttl, '600');
  console.log('voice started push: ok');

  // Log out on bob's device: the worker and its subscription go.
  const b2 = await b.context().newPage();
  b2.on('pageerror', (e) => bob.errors.push(`bob: ${e}`));
  await b2.goto(`${ORIGIN}/`);
  await b2.getByRole('navigation', { name: 'Rooms' }).waitFor();
  await logout(b2);
  await b2.waitForURL(/\/login/);
  assert.equal(await b2.evaluate(() => navigator.serviceWorker.getRegistration().then((r) => !!r)), false, 'worker after logout');
  console.log('logout removes the worker: ok');

  await sleep(300);
  assert.equal(got.length, 0, 'unexpected pushes');
  assert.deepEqual(await cspViolations(a), [], 'CSP violations');
  assert.deepEqual([...alice.errors, ...bob.errors], [], 'page errors');
  console.log('e2e push: PASS');
} finally {
  await stop();
  fake.close();
}
