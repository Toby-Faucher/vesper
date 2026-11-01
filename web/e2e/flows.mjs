// UI flows test. Starts the release server like two-browser.mjs and checks, in real Chromium: the invite code leaves
// the address bar, a bad link, autocomplete attributes, passwords cleared and focus on the error after a failed
// submit, routes, a room link surviving sign in, the call staying up while moving between rooms and settings,
// force-mute (also on rejoin and from the person card), a kick ending the call, and a room that becomes private ending
// the call and leaving bob's search.
// Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { ORIGIN, browser, cli, composer, cspViolations, inVoice, logout, openRoom, roomId, roomLink, searchFor, searchHits, signedIn, sleep, startServer } from './lib.mjs';

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
// The notice in the live call bar; the inert copy that fades out after Leave is aria-hidden, so the region query skips it.
const mutedNotice = (page) => page.getByRole('region', { name: 'Call' }).getByText(MUTED_NOTICE);

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

  // The switcher: Ctrl+K works from the composer, a typed room name and Enter open that room, Esc closes and gives focus
  // back. Alt+Down and Alt+Up step through the rooms; Ctrl+\ hides the sidebar and focus moves to the Show button.
  await openRoom(a, 'general');
  const msgBox = composer(a);
  // The switcher's chunk fails to load the first time: the dialog says so, and the next Ctrl+K tries again. While the
  // second load is held back, keys typed go into the dialog's field (not the composer) and are kept when it loads.
  const chunk = '**/assets/switcher-chunk-*.js';
  await a.route(chunk, (r) => r.abort());
  await msgBox.focus();
  await a.keyboard.press('Control+k');
  await a.getByRole('alert').filter({ hasText: 'Could not load the switcher.' }).waitFor();
  await a.unroute(chunk);
  let release;
  const held = new Promise((r) => (release = r));
  await a.route(chunk, async (r) => { await held; await r.continue(); });
  await a.keyboard.press('Control+k');
  await a.getByText('Loading...').waitFor();
  await a.keyboard.type('mus');
  release();
  const box = a.getByRole('combobox', { name: 'Switch to' });
  await box.waitFor();
  await a.unroute(chunk);
  assert.equal(await box.inputValue(), 'mus', 'the text typed while the switcher loaded was lost');
  assert.equal(await msgBox.inputValue(), '', 'keys typed while the switcher loaded went into the composer');
  assert.ok(await box.evaluate((e) => e === document.activeElement), 'the switcher field has no focus');
  assert.equal(await a.getByRole('option', { selected: true }).innerText().then((t) => t.split('\n')[0]), 'music');
  await a.keyboard.press('Enter');
  await a.waitForURL(`${ORIGIN}/r/${music.id}`);
  await a.getByRole('heading', { level: 1, name: 'music', exact: true }).waitFor();
  await box.waitFor({ state: 'hidden' });
  await msgBox.focus();
  await a.keyboard.press('Control+k');
  await box.waitFor();
  assert.equal(await box.inputValue(), '', 'the switcher kept the last query');
  await a.keyboard.press('Escape');
  await box.waitFor({ state: 'hidden' });
  assert.ok(await msgBox.evaluate((e) => e === document.activeElement), 'focus did not go back to the composer');
  // `p ` scopes to people; a person opens their card as a dialog.
  await a.keyboard.press('Control+k');
  await box.fill('p bob');
  assert.equal(await a.getByRole('option').count(), 1, 'p scoped to people');
  await a.keyboard.press('Enter');
  await a.getByRole('dialog', { name: 'bob' }).waitFor();
  await a.keyboard.press('Escape');
  await a.getByRole('dialog', { name: 'bob' }).waitFor({ state: 'hidden' });
  assert.ok(await msgBox.evaluate((e) => e === document.activeElement), 'focus did not go back after the person card');
  // The room above music in the sidebar, whichever it is, then back. In the composer Alt+Up is the field's (a Mac moves
  // the caret with it); Ctrl+Alt+Up works there.
  const above = await a.getByRole('navigation', { name: 'Rooms' }).getByRole('link').evaluateAll((l) => l.map((x) => x.getAttribute('href')));
  await a.keyboard.press('Alt+ArrowUp');
  await sleep(200);
  assert.equal(a.url(), `${ORIGIN}/r/${music.id}`, 'Alt+Up in the composer changed the room');
  await a.keyboard.press('Control+Alt+ArrowUp');
  await a.waitForURL(`${ORIGIN}${above[above.indexOf(`/r/${music.id}`) - 1]}`);
  await a.locator('main').focus();
  await a.keyboard.press('Alt+ArrowDown');
  await a.getByRole('heading', { level: 1, name: 'music', exact: true }).waitFor();
  await a.getByRole('button', { name: 'Hide sidebar' }).focus();
  await a.keyboard.press('Control+Backslash');
  await a.getByRole('navigation', { name: 'Rooms' }).waitFor({ state: 'hidden' });
  const show = a.getByRole('button', { name: 'Show sidebar' });
  await a.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Show sidebar');
  await show.click();
  await signedIn(a);
  // Ctrl+Shift+M and D are the browser's (bookmark all tabs, and others) unless alice is in a call: only then the page
  // takes them. A listener added after the app's sees whether it called preventDefault.
  const prevented = (key) => a.evaluate((key) => new Promise((done) => {
    addEventListener('keydown', (e) => done(e.defaultPrevented), { once: true });
    dispatchEvent(new KeyboardEvent('keydown', { key, code: `Key${key.toUpperCase()}`, ctrlKey: true, shiftKey: true, cancelable: true }));
  }), key);
  assert.equal(await prevented('M'), false, 'Ctrl+Shift+M taken from the browser outside a call');
  assert.equal(await prevented('D'), false, 'Ctrl+Shift+D taken from the browser outside a call');
  await join(a, 'general');
  assert.equal(await prevented('M'), true, 'Ctrl+Shift+M not handled in a call');
  await a.getByRole('button', { name: 'Unmute' }).waitFor();
  assert.equal(await prevented('M'), true, 'Ctrl+Shift+M not handled in a call');
  await a.getByRole('button', { name: 'Mute' }).waitFor();
  await a.getByRole('button', { name: 'Leave' }).click();
  await a.getByRole('button', { name: 'Join' }).waitFor();
  console.log('switcher and shortcuts: ok');

  // Search opens from the keys: its chunk fails to load the first time (the dialog says so) and Ctrl+Shift+F tries
  // again; text typed while the second load is held back is kept, and Esc gives focus back to the composer.
  const searchChunk = '**/assets/SearchDialog-*.js';
  await a.route(searchChunk, (r) => r.abort());
  await composer(a).focus();
  await a.keyboard.press('Control+Shift+F');
  await a.getByRole('alert').filter({ hasText: 'Could not load search.' }).waitFor();
  await a.unroute(searchChunk);
  let letGo;
  const hold = new Promise((r) => (letGo = r));
  await a.route(searchChunk, async (r) => { await hold; await r.continue(); });
  await a.keyboard.press('Control+Shift+F');
  await a.getByRole('dialog', { name: 'Search' }).getByText('Loading...').waitFor();
  await a.keyboard.type('hel');
  letGo();
  const searchBox = a.getByRole('combobox', { name: 'Search messages' });
  await searchBox.waitFor();
  await a.unroute(searchChunk);
  assert.equal(await searchBox.inputValue(), 'hel', 'the text typed while search loaded was lost');
  assert.equal(await composer(a).inputValue(), '', 'keys typed while search loaded went into the composer');
  await a.keyboard.press('Escape');
  await searchBox.waitFor({ state: 'hidden' });
  assert.ok(await composer(a).evaluate((e) => e === document.activeElement), 'focus did not go back to the composer');
  // The switcher's action opens it too, empty.
  await a.keyboard.press('Control+k');
  await a.getByRole('combobox', { name: 'Switch to' }).fill('> search messages');
  await a.keyboard.press('Enter');
  await searchBox.waitFor();
  assert.equal(await searchBox.inputValue(), '', 'search kept the last query');
  await a.keyboard.press('Escape');
  await searchBox.waitFor({ state: 'hidden' });
  console.log('search opens from the keys and the switcher: ok');

  // A room link opened signed out: sign in returns to that room (music), not to the first one; Back does not loop to
  // sign in. A room id nobody can see gives the plain missing state after sign in.
  const signInAs = async (page, path) => {
    await page.goto(`${ORIGIN}${path}`);
    await page.waitForURL(`${ORIGIN}/login?next=${encodeURIComponent(path)}`);
    await page.getByLabel('Username').fill('alice');
    await page.getByLabel('Password').fill('test-password-1');
    await page.getByRole('button', { name: 'Sign in' }).click();
    await page.waitForURL(`${ORIGIN}${path}`);
  };
  await logout(a);
  await a.getByRole('button', { name: 'Sign in' }).waitFor();
  await signInAs(a, `/r/${music.id}`);
  await a.getByRole('heading', { name: 'music' }).waitFor();
  await logout(a);
  await a.getByRole('button', { name: 'Sign in' }).waitFor();
  await signInAs(a, '/r/99999');
  await a.getByText('This room is not here.').waitFor();
  console.log('room link survives sign in: ok');

  // Force-mute disables Mute, and still holds after Leave and a rejoin. Lifting it enables Mute again.
  await join(b, 'general');
  await call(a, 'POST', `/api/users/${bobId}/force-mute`, { muted: true });
  await mutedNotice(b).waitFor();
  assert.ok(await b.getByRole('button', { name: 'Unmute' }).isDisabled(), 'Mute not disabled when force-muted');
  await b.getByRole('button', { name: 'Leave' }).click();
  // The bar and its notice go at once (an inert, aria-hidden copy fades out for --dur-3; role queries skip it).
  assert.equal(await b.getByRole('region', { name: 'Call' }).count(), 0, 'the call bar is still there after Leave');
  await join(b, 'general');
  await mutedNotice(b).waitFor();
  await call(a, 'POST', `/api/users/${bobId}/force-mute`, { muted: false });
  await mutedNotice(b).waitFor({ state: 'detached' });
  assert.ok(await b.getByRole('button', { name: 'Unmute' }).isEnabled(), 'Mute still disabled after the lift');
  console.log('force-mute: ok');

  // The same from the person card: alice (owner) opens bob from the sidebar presence list.
  await inVoice(a, 'bob').getByRole('button').click();
  const card = a.getByRole('dialog', { name: 'bob' });
  // While the request runs the action is busy: a second click sends nothing.
  const sent = [];
  a.on('request', (r) => { if (r.method() === 'POST' && r.url().includes(`/api/users/${bobId}/`)) sent.push(r.url().split('/').at(-1)); });
  await a.route('**/force-mute', async (r) => { await sleep(500); await r.continue(); });
  const muteBtn = card.getByRole('button', { name: 'Mute for everyone' });
  await muteBtn.click();
  assert.equal(await muteBtn.getAttribute('aria-disabled'), 'true', 'the busy action is not aria-disabled');
  await muteBtn.click({ force: true }); // force: Playwright would wait for aria-disabled to clear
  await mutedNotice(b).waitFor();
  await a.unroute('**/force-mute');
  assert.deepEqual(sent, ['force-mute'], 'a busy action sent a second request');
  await card.getByRole('button', { name: 'Allow to speak' }).click();
  await mutedNotice(b).waitFor({ state: 'detached' });
  // Disconnect asks first and sends nothing until confirmed; Cancel keeps bob connected.
  await card.getByRole('button', { name: 'Disconnect' }).click();
  const confirm = a.getByRole('dialog', { name: 'Disconnect bob?' });
  await confirm.waitFor();
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  await confirm.waitFor({ state: 'hidden' });
  assert.deepEqual(sent.filter((x) => x === 'disconnect'), [], 'Disconnect called the server before the confirm');
  await a.keyboard.press('Escape');
  await card.waitFor({ state: 'hidden' });
  assert.ok(await inVoice(a, 'bob').getByRole('button').evaluate((e) => e === document.activeElement), 'focus did not go back to the person');
  console.log('person card force-mute, busy and disconnect confirm: ok');

  // A kick, from the card and without a confirm, sends peer_left with bob's own id: his call ends, the socket stays.
  await inVoice(a, 'bob').getByRole('button').click();
  await card.getByRole('button', { name: 'Remove from voice' }).click();
  await card.waitFor({ state: 'hidden' });
  assert.deepEqual(sent.slice(-1), ['kick'], 'Remove from voice did not call the server at once');
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
  const why = (await b.getByRole('alert').first().textContent()).trim(); // Status puts a space after its icon
  assert.match(why, /^(You no longer have access to that room\.|Your call ended\.)/);
  assert.equal(await roomLink(b, 'secret').count(), 0, 'secret still listed');
  assert.notEqual(roomId(b), room.id, 'still on the hidden room');
  console.log(`vanished room ends the call: ok (${why})`);

  // Search follows the change: a message in the room bob just lost never shows for him, while alice finds it.
  await call(a, 'POST', `/api/rooms/${room.id}/messages`, { body: 'zebra plans for the staff', reply_to: null, attachments: [] });
  await searchFor(a, 'zebra', { keys: true }); // alice is not in a room here: the keys work anywhere
  await searchHits(a).first().waitFor({ timeout: 5000 });
  await searchFor(b, 'zebra');
  await b.getByText('No messages found.').waitFor({ timeout: 5000 });
  assert.equal(await searchHits(b).count(), 0, 'bob found a message from a room he cannot see');
  for (const p of [a, b]) await p.keyboard.press('Escape');
  console.log('search skips a room that became private: ok');

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
