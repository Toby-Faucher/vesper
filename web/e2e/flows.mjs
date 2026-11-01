// UI flows test. Starts the release server like two-browser.mjs and checks, in real Chromium: the invite code leaves
// the address bar, a bad link, autocomplete attributes, passwords cleared and focus on the error after a failed
// submit, routes, the call staying up while moving between rooms and settings, force-mute (also on rejoin), a kick
// ending the call, and a room that becomes private ending the call.
// Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { ORIGIN, browser, cli, cspViolations, inVoice, logout, openRoom, roomId, roomLink, signedIn, startServer } from './lib.mjs';

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

async function join(page, room) {
  await openRoom(page, room);
  await page.getByRole('button', { name: 'Join' }).click();
  await page.getByRole('button', { name: 'Leave' }).waitFor();
}
const focusedText = (page) => page.evaluate(() => document.activeElement?.textContent?.trim());
const MUTED_NOTICE = 'A moderator muted you.';

const stop = await startServer();
try {
  const alice = await browser('alice', []);
  const a = alice.page;

  // A bad link: plain text, and the fragment is gone.
  await a.goto(`${ORIGIN}/invite#not-a-code`);
  await a.getByText('This invite link is not valid. Ask for a new one.').waitFor();
  assert.equal(new URL(a.url()).hash, '', 'bad invite fragment stayed in the URL');

  const ownerLink = await cli(['create-owner']);
  await a.goto(`${ORIGIN}/login`); // from /invite, the link alone would be a same-page fragment change
  await a.goto(ownerLink);
  await a.getByLabel('Username').waitFor();
  assert.equal(a.url(), `${ORIGIN}/invite`, 'invite code stayed in the address bar');
  for (const [label, want] of [['Username', 'username'], ['Display name', 'nickname'], ['Password', 'new-password']])
    assert.equal(await a.getByLabel(label).getAttribute('autocomplete'), want, `${label} autocomplete`);
  await a.getByLabel('Username').fill('alice');
  await a.getByLabel('Display name').fill('alice');
  await a.getByLabel('Password').fill('test-password-1');
  await a.getByRole('button', { name: 'Create account' }).click();
  await signedIn(a);
  await a.waitForURL(/\/r\/\d+$/); // `/` opens the first room
  console.log('invite fragment cleared: ok');

  // Routes: an unknown path is a plain not-found page; a room id nobody can see says so; `/` opens the last room.
  await a.goto(`${ORIGIN}/no-such-page`);
  await a.getByText('There is no page at this address.').waitFor();
  await a.goto(`${ORIGIN}/r/99999`);
  await a.getByText('This room is not here.').waitFor();
  await a.goto(`${ORIGIN}/`);
  await a.waitForURL(/\/r\/\d+$/);
  console.log('routes: ok');

  // Wrong password: plain text, the password field is empty, focus is on the error.
  await logout(a);
  await a.getByRole('button', { name: 'Sign in' }).waitFor();
  assert.equal(await a.getByLabel('Username').getAttribute('autocomplete'), 'username');
  assert.equal(await a.getByLabel('Password').getAttribute('autocomplete'), 'current-password');
  await a.getByLabel('Username').fill('alice');
  await a.getByLabel('Password').fill('test-password-2');
  await a.getByRole('button', { name: 'Sign in' }).click();
  await a.getByText('Wrong username or password.').waitFor();
  assert.equal(await a.getByLabel('Password').inputValue(), '', 'password kept after a failed sign in');
  assert.equal(await focusedText(a), 'Wrong username or password.', 'focus did not move to the error');
  await a.getByLabel('Password').fill('test-password-1');
  await a.getByRole('button', { name: 'Sign in' }).click();
  await signedIn(a);
  console.log('failed sign in: ok');

  // A failed signup (name taken) clears the password too.
  const bobLink = (await call(a, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 })).url;
  const bob = await browser('bob', []);
  const b = bob.page;
  await b.goto(bobLink);
  await b.getByLabel('Username').fill('alice');
  await b.getByLabel('Display name').fill('bob');
  await b.getByLabel('Password').fill('test-password-1');
  await b.getByRole('button', { name: 'Create account' }).click();
  await b.getByText('That name is already taken.').waitFor();
  assert.equal(await b.getByLabel('Password').inputValue(), '', 'password kept after a failed signup');
  assert.equal(await focusedText(b), 'That name is already taken.');
  await b.getByLabel('Username').fill('bob');
  await b.getByLabel('Password').fill('test-password-1');
  await b.getByRole('button', { name: 'Create account' }).click();
  await signedIn(b);
  const bobId = (await call(b, 'GET', '/api/me')).user.user_id;
  console.log('failed signup: ok');

  // The call is held by the shell, not the page: moving to another room and to Settings keeps it up.
  const music = await call(a, 'POST', '/api/rooms', { name: 'music' });
  await join(b, 'general');
  const general = roomId(b);
  const pcUp = () => b.evaluate(() => window.__pcs.at(-1)?.connectionState);
  await roomLink(b, 'music').waitFor();
  await openRoom(b, 'music');
  assert.equal(roomId(b), music.id);
  await b.getByRole('button', { name: 'Join voice' }).waitFor(); // not in this room's call
  await b.getByRole('button', { name: 'Settings' }).click();
  await b.waitForURL(`${ORIGIN}/settings/voice`);
  await b.getByLabel('Bitrate', { exact: false }).first().waitFor();
  await b.goBack();
  await b.waitForURL(`${ORIGIN}/r/${music.id}`);
  await openRoom(b, 'general');
  await b.getByRole('button', { name: 'Leave' }).waitFor();
  assert.equal(await pcUp(), 'connected', 'the call dropped while moving between pages');
  assert.equal(await b.getByText('Your call ended.').count(), 0);
  await inVoice(a, 'bob').waitFor(); // alice still sees bob in voice
  assert.equal(roomId(b), general);
  await b.getByRole('button', { name: 'Leave' }).click();
  await inVoice(a, 'bob').waitFor({ state: 'detached' });
  console.log('call survives navigation: ok');

  // Force-mute disables Mute, and still holds after Leave and a rejoin. Lifting it enables Mute again.
  await join(b, 'general');
  await call(a, 'POST', `/api/users/${bobId}/force-mute`, { muted: true });
  await b.getByText(MUTED_NOTICE).waitFor();
  assert.ok(await b.getByRole('button', { name: 'Unmute' }).isDisabled(), 'Mute not disabled when force-muted');
  await b.getByRole('button', { name: 'Leave' }).click();
  assert.equal(await b.getByText(MUTED_NOTICE).count(), 0, 'force-mute notice stayed after Leave');
  await join(b, 'general');
  await b.getByText(MUTED_NOTICE).waitFor();
  await call(a, 'POST', `/api/users/${bobId}/force-mute`, { muted: false });
  await b.getByText(MUTED_NOTICE).waitFor({ state: 'detached' });
  assert.ok(await b.getByRole('button', { name: 'Unmute' }).isEnabled(), 'Mute still disabled after the lift');
  console.log('force-mute: ok');

  // A kick sends peer_left with bob's own id: his call ends, the socket stays.
  await call(a, 'POST', `/api/users/${bobId}/kick`);
  await b.getByText('Your call ended.', { exact: false }).waitFor();
  await b.getByRole('button', { name: 'Join' }).waitFor();
  console.log('kick ends the call: ok');

  // A room bob can no longer see ends his call.
  const room = await call(a, 'POST', '/api/rooms', { name: 'secret' });
  await roomLink(b, 'secret').waitFor();
  await join(b, 'secret');
  await call(a, 'PATCH', `/api/rooms/${room.id}`, { restricted: true });
  // The server takes bob out of voice too, so its peer_left may come before rooms_changed: either text ends the call.
  await b.getByRole('button', { name: 'Join' }).waitFor();
  const why = await b.getByRole('alert').first().textContent();
  assert.match(why, /^(You no longer have access to that room\.|Your call ended\.)/);
  assert.equal(await roomLink(b, 'secret').count(), 0, 'secret still listed');
  assert.notEqual(roomId(b), room.id, 'still on the hidden room');
  console.log(`vanished room ends the call: ok (${why})`);

  // The denoiser (worklet, worker, WASM) under the server's CSP. A CSP block in the worker is not a page console
  // message: it shows as the fallback reason in the status line. Runs only when dist/dsp/ was built (not in CI).
  const pages = [alice.page, bob.page];
  if (existsSync(new URL('../dist/dsp/vesper_dsp_bg.wasm', import.meta.url))) {
    const dsp = await browser('alice-dsp', []);
    const d = dsp.page;
    await d.addInitScript(() => localStorage.setItem('vesper.audio', JSON.stringify({ mode: 'deepfilter' })));
    await d.goto(`${ORIGIN}/login`);
    await d.getByLabel('Username').fill('alice');
    await d.getByLabel('Password').fill('test-password-1');
    await d.getByRole('button', { name: 'Sign in' }).click();
    await signedIn(d);
    await join(d, 'general');
    const line = d.getByText('Noise suppression:').first();
    let status = '';
    for (let i = 0; i < 80 && (i < 8 || status.includes('loading')); i++) {
      await new Promise((r) => setTimeout(r, 250));
      status = await line.innerText();
    }
    assert.equal(status, 'Noise suppression: deepfilter', 'denoiser did not start under the CSP');
    pages.push(d);
    alice.errors.push(...dsp.errors);
    console.log('denoiser under CSP: ok');
  } else console.log('denoiser under CSP: skipped (no dist/dsp; run scripts/build-dsp.sh and npm run build)');
  for (const p of pages) assert.deepEqual(await cspViolations(p), [], 'CSP violations');
  console.log('no CSP violations: ok');
  assert.deepEqual([...alice.errors, ...bob.errors], [], 'page errors');
  console.log('e2e flows: PASS');
} finally {
  await stop();
}
