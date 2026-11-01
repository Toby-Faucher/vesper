// UI flows test. Starts the release server like two-browser.mjs and checks, in real Chromium: the invite code leaves
// the address bar, a bad link, autocomplete attributes, passwords cleared and focus on the error after a failed
// submit, routes, a room link surviving sign in, the call staying up while moving between rooms and settings,
// force-mute (also on rejoin and from the person card), a kick ending the call, and a room that becomes private ending
// the call and leaving bob's search; Settings > Account signing out a second device and changing the password.
// Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { ORIGIN, browser, cli, composer, cspViolations, inVoice, logout, openRoom, roomId, roomLink, searchFor, searchHits, signedIn, signup, sleep, startServer } from './lib.mjs';

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

const BAD_LINK = 'This invite link does not work. Ask the person who sent it for a new one.';

// Headless Chromium answers 'denied' without asking and shows nothing, so the page gets a stand-in Notification that
// starts at 'default', counts the asks, grants, and records each notification. The grant lasts the tab (sessionStorage),
// as a real one would over a reload. Add it before the page loads.
const fakeNotifications = () => {
  let p = sessionStorage.getItem('fake-permission') ?? 'default';
  window.__asked = 0;
  window.__shown = [];
  window.__notes = [];
  window.Notification = class {
    static get permission() { return p; }
    static async requestPermission() {
      window.__asked++;
      sessionStorage.setItem('fake-permission', (p = 'granted'));
      return p;
    }
    constructor(title, o = {}) { window.__shown.push({ title, body: o.body, tag: o.tag }); window.__notes.push(this); }
    close() { this.closed = true; }
  };
};

// The call sounds are the app's only oscillators: record each tone's pitch as it starts, and let it play on.
const recordTones = () => {
  window.__tones = [];
  const start = OscillatorNode.prototype.start;
  OscillatorNode.prototype.start = function (...a) { window.__tones.push(Math.round(this.frequency.value)); return start.apply(this, a); };
};
// The pitches of each cue, as in web/src/lib/ui/chime.ts.
const CUE = { connect: [587, 880], disconnect: [880, 587], mute: [1319, 880], unmute: [880, 1319] };

// The three welcome steps once, with Back: the step count, the heading taking focus, the colour saved, and the browser
// asked about notifications only from the button.
async function walkWelcome(page) {
  await page.waitForURL(`${ORIGIN}/welcome`);
  const focusedHeading = () => page.evaluate(() => document.activeElement?.tagName === 'H1' && document.activeElement.textContent);
  await page.getByText('Step 1 of 3').waitFor();
  assert.equal(await focusedHeading(), 'How others see you');
  await page.getByRole('radio', { name: 'Teal' }).check();
  await page.getByRole('button', { name: 'Next' }).click();
  await page.getByText('Step 2 of 3').waitFor();
  assert.equal(await focusedHeading(), 'Check your microphone');
  assert.equal((await call(page, 'GET', '/api/me')).user.accent, '#31c3b5', 'colour not saved');
  await page.getByRole('button', { name: 'Back' }).click();
  assert.equal(await focusedHeading(), 'How others see you');
  assert.ok(await page.getByRole('radio', { name: 'Teal' }).isChecked(), 'Back lost the colour');
  await page.getByRole('button', { name: 'Next' }).click();
  await page.getByRole('button', { name: 'Mic test (hear yourself)' }).click();
  await page.waitForFunction(() => Number(document.querySelector('[role=meter]')?.getAttribute('aria-valuenow')) > -100);
  await page.getByRole('button', { name: 'Next' }).click();
  await page.getByText('Step 3 of 3').waitFor();
  assert.equal(await focusedHeading(), 'Mentions');
  assert.equal(await page.evaluate(() => window.__asked), 0, 'notification permission asked before the click');
  await page.getByRole('button', { name: 'Turn on notifications' }).click();
  await page.getByText('Notifications are on.', { exact: false }).waitFor();
  assert.equal(await page.evaluate(() => window.__asked), 1);
  await page.getByRole('button', { name: 'Open Vesper' }).click();
  await signedIn(page);
  await page.waitForURL(/\/r\/\d+$/);
  console.log('welcome steps: ok');
}

const stop = await startServer();
try {
  const alice = await browser('alice', []);
  const a = alice.page;

  // A bad link: plain text, and the fragment is gone.
  await a.goto(`${ORIGIN}/invite#not-a-code`);
  await a.getByText(BAD_LINK).waitFor();
  assert.equal(new URL(a.url()).hash, '', 'bad invite fragment stayed in the URL');

  // A second link opened in the same tab changes only the fragment (no page load): the form takes the new code.
  const ownerLink = await cli(['create-owner']);
  await a.goto(ownerLink);
  await a.getByLabel('Username').waitFor();
  assert.equal(a.url(), `${ORIGIN}/invite`, 'invite code stayed in the address bar');
  console.log('second invite link in the same tab: ok');
  for (const [label, want] of [['Username', 'username'], ['Display name', 'nickname'], ['Password', 'new-password']])
    assert.equal(await a.getByLabel(label, { exact: true }).getAttribute('autocomplete'), want, `${label} autocomplete`);
  await a.getByLabel('Username').fill('alice');
  await a.getByLabel('Display name').fill('alice');
  await a.getByLabel('Password', { exact: true }).fill('test-password-1');
  await a.getByRole('button', { name: 'Create account' }).click();
  // Signup lands on the welcome steps. A stray Esc in the name field keeps the page and the typed name; Skip opens a room.
  await a.waitForURL(`${ORIGIN}/welcome`);
  await a.getByRole('heading', { name: 'How others see you' }).waitFor();
  await a.getByLabel('Display name').fill('alice a');
  await a.getByLabel('Display name').press('Escape');
  await sleep(300);
  assert.equal(new URL(a.url()).pathname, '/welcome', 'Esc in the name field left the welcome steps');
  assert.equal(await a.getByLabel('Display name').inputValue(), 'alice a', 'Esc lost the typed name');
  await a.getByRole('button', { name: 'Skip for now' }).click();
  await signedIn(a);
  await a.waitForURL(/\/r\/\d+$/);
  console.log('invite fragment cleared, Esc keeps the welcome, Skip leaves it: ok');

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
  assert.equal(await a.getByLabel('Password', { exact: true }).getAttribute('autocomplete'), 'current-password');
  await a.getByLabel('Username').fill('alice');
  await a.getByLabel('Password', { exact: true }).fill('test-password-2');
  // The toggle: name and visible text stay "Show", aria-pressed carries the state; a failed submit hides the password again.
  const showPw = a.getByRole('button', { name: 'Show password' });
  await showPw.click();
  assert.deepEqual([await showPw.textContent(), await showPw.getAttribute('aria-pressed')], ['Show', 'true']);
  assert.equal(await a.getByLabel('Password', { exact: true }).getAttribute('type'), 'text');
  await a.getByRole('button', { name: 'Sign in' }).click();
  await a.getByText('Wrong username or password.').waitFor();
  assert.equal(await a.getByLabel('Password', { exact: true }).inputValue(), '', 'password kept after a failed sign in');
  assert.equal(await a.getByLabel('Password', { exact: true }).getAttribute('type'), 'password', 'password left visible after a failed sign in');
  assert.equal(await showPw.getAttribute('aria-pressed'), 'false');
  assert.equal(await focusedText(a), 'Wrong username or password.', 'focus did not move to the error');
  await a.getByLabel('Password', { exact: true }).fill('test-password-1');
  await a.getByRole('button', { name: 'Sign in' }).click();
  await signedIn(a);
  console.log('failed sign in: ok');

  // A failed signup (name taken) clears the password too.
  const bobLink = (await call(a, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 })).url;
  const bob = await browser('bob', []);
  const b = bob.page;
  await b.addInitScript(fakeNotifications);
  await b.addInitScript(recordTones);
  await b.goto(bobLink);
  await b.getByLabel('Username').fill('alice');
  await b.getByLabel('Display name').fill('bob');
  await b.getByLabel('Password', { exact: true }).fill('test-password-1');
  await b.getByRole('button', { name: 'Show password' }).click();
  await b.getByRole('button', { name: 'Create account' }).click();
  await b.getByText('That name is already taken.').waitFor();
  assert.equal(await b.getByLabel('Password', { exact: true }).inputValue(), '', 'password kept after a failed signup');
  assert.equal(await b.getByLabel('Password', { exact: true }).getAttribute('type'), 'password', 'password left visible after a failed signup');
  assert.equal(await focusedText(b), 'That name is already taken.');
  await b.getByLabel('Username').fill('bob');
  await b.getByLabel('Password', { exact: true }).fill('test-password-1');
  await b.getByRole('button', { name: 'Create account' }).click();
  await walkWelcome(b);
  const bobId = (await call(b, 'GET', '/api/me')).user.user_id;
  console.log('failed signup: ok');

  // A used link is found only on submit: one plain message, no form, focus on it.
  await a.goto(bobLink);
  await a.getByLabel('Username').fill('carol');
  await a.getByLabel('Display name').fill('carol');
  await a.getByLabel('Password', { exact: true }).fill('test-password-1');
  await a.getByRole('button', { name: 'Create account' }).click();
  await a.getByText(BAD_LINK).waitFor();
  assert.equal(await focusedText(a), BAD_LINK, 'focus did not move to the bad link message');
  assert.equal(await a.getByLabel('Username').count(), 0, 'the form stayed after a used link');
  await a.goto(`${ORIGIN}/`);
  await signedIn(a);
  console.log('used invite link: ok');

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
  // Push to talk in Settings: Control is refused with a plain line and the key stays; Q is taken, shown as Q, and the call
  // bar says so. The input meter is a role=meter with a number, on a track you can see.
  await b.getByLabel('Push to talk').check();
  const keyBtn = b.getByRole('button', { name: /^(Key:|Press a key)/ });
  await keyBtn.click();
  await b.getByText('Press a key...').waitFor();
  // The waiting line is a status, read with the button.
  await b.getByRole('status').filter({ hasText: 'Press the key to use. Esc cancels.' }).waitFor();
  assert.match(await keyBtn.getAttribute('aria-describedby'), /\bptt-capture\b/);
  for (const k of ['Enter', 'Backspace']) {
    await b.keyboard.press(k);
    await b.getByRole('alert').filter({ hasText: 'Tab, Enter and Backspace move around the app and type.' }).waitFor({ timeout: 2000 });
  }
  // Tab is not taken: it leaves the button and ends the capture.
  await b.keyboard.press('Tab');
  await b.getByRole('button', { name: 'Key: Space' }).waitFor({ timeout: 2000 });
  assert.equal(await keyBtn.evaluate((el) => el === document.activeElement), false, 'Tab stayed on the key button');
  await keyBtn.click();
  await b.keyboard.press('Control');
  await b.getByRole('alert').filter({ hasText: 'Pick a key that is not Ctrl, Alt, Shift or Meta.' }).waitFor({ timeout: 2000 });
  await b.keyboard.press('KeyK');
  await b.getByRole('alert').filter({ hasText: 'A keyboard shortcut uses that key.' }).waitFor({ timeout: 2000 });
  assert.equal(await b.evaluate(() => JSON.parse(localStorage.getItem('vesper.audio')).pushToTalk.key), 'Space', 'a refused key was saved');
  await b.keyboard.press('KeyQ');
  await b.getByRole('button', { name: 'Key: Q' }).waitFor({ timeout: 2000 });
  assert.equal(await b.evaluate(() => JSON.parse(localStorage.getItem('vesper.audio')).pushToTalk.key), 'KeyQ');
  assert.ok(await keyBtn.evaluate((el) => el === document.activeElement), 'focus left the key button after the capture');
  await b.getByRole('region', { name: 'Call' }).getByText('Push to talk: hold Q').waitFor({ timeout: 2000 });
  await keyBtn.click();
  await b.keyboard.press('Escape'); // Esc cancels the capture, not Settings
  await b.getByRole('button', { name: 'Key: Q' }).waitFor({ timeout: 2000 });
  assert.equal(new URL(b.url()).pathname, '/settings/voice', 'Esc in the key capture closed Settings');
  await b.getByLabel('Push to talk').uncheck();
  const meter = await b.getByRole('meter', { name: 'Input level' }).evaluate((m) => {
    const cs = getComputedStyle(m);
    return { now: m.getAttribute('aria-valuenow'), text: m.getAttribute('aria-valuetext'), h: m.getBoundingClientRect().height, track: cs.backgroundColor !== 'rgba(0, 0, 0, 0)' && cs.boxShadow !== 'none' };
  });
  assert.ok(Number.isFinite(Number(meter.now)) && meter.now !== null, `meter value ${meter.now}`);
  assert.match(meter.text, /^(Silent|-?\d+ dBFS)$/);
  assert.ok(meter.h >= 6 && meter.track, `meter track not visible: ${JSON.stringify(meter)}`);
  await b.getByText(/^(Silent|-?\d+ dBFS)$/).first().waitFor(); // the number beside the bar
  console.log('settings push-to-talk key and meter: ok');
  // Close goes back to the room it came from, and Back after it is the room before: inside Settings a section link, the
  // sidebar's Settings button and the call bar's Call details each replace the entry.
  await b.getByRole('navigation', { name: 'Settings sections' }).getByRole('link', { name: 'Diagnostics' }).click();
  await b.getByRole('heading', { name: 'Diagnostics', level: 2 }).waitFor();
  await b.getByRole('button', { name: 'Settings' }).click();
  await b.getByRole('heading', { name: 'Voice', level: 2 }).waitFor();
  await b.getByRole('region', { name: 'Call' }).getByRole('link', { name: 'Call details' }).click();
  await b.getByRole('heading', { name: 'Diagnostics', level: 2 }).waitFor();
  await b.getByRole('button', { name: 'Close' }).click();
  await b.waitForURL(`${ORIGIN}/r/${music.id}`);
  await b.goBack();
  await b.waitForURL(`${ORIGIN}/r/${general}`);
  await b.goForward();
  await b.waitForURL(`${ORIGIN}/r/${music.id}`);
  console.log('settings close goes back: ok');
  await openRoom(b, 'general');
  await b.getByRole('button', { name: 'Leave' }).waitFor();
  assert.equal(await pcUp(), 'connected', 'the call dropped while moving between pages');
  assert.equal(await b.getByText('Your call ended.').count(), 0);
  await inVoice(a, 'bob').waitFor(); // alice still sees bob in voice
  assert.equal(roomId(b), general);
  await b.getByRole('button', { name: 'Leave' }).click();
  await inVoice(a, 'bob').waitFor({ state: 'detached' });
  console.log('call survives navigation: ok');

  // Call sounds, on by default for my own call: join, mute, unmute and leave each play their cue once, in order.
  await b.evaluate(() => (window.__tones = []));
  await join(b, 'general');
  await b.getByRole('button', { name: 'Mute', exact: true }).click();
  await b.getByRole('button', { name: 'Unmute' }).click();
  await b.getByRole('button', { name: 'Mute', exact: true }).waitFor();
  await b.getByRole('button', { name: 'Leave' }).click();
  await inVoice(a, 'bob').waitFor({ state: 'detached' });
  await sleep(300);
  assert.deepEqual(await b.evaluate(() => window.__tones), [...CUE.connect, ...CUE.mute, ...CUE.unmute, ...CUE.disconnect], 'call sound sequence');
  console.log('call sounds: ok');

  // A Voice setting persists over a reload. Settings opened from its address closes to the last room, in place, and Esc
  // closes it too.
  await b.goto(`${ORIGIN}/settings/voice`);
  await b.getByLabel('Auto gain').check();
  // A second tab, as a reload would: b's page writes mode=browser into storage on every load (lib.mjs), a new page does not.
  const b2 = await b.context().newPage();
  await b2.goto(`${ORIGIN}/settings/voice`);
  await b2.getByLabel('Auto gain').waitFor();
  assert.ok(await b2.getByLabel('Auto gain').isChecked(), 'Auto gain did not persist');
  await b2.close();
  await b.getByLabel('Auto gain').uncheck();
  assert.equal(await b.evaluate(() => JSON.parse(localStorage.getItem('vesper.audio')).autoGain), false);
  // The noise gate switch follows the saved mode, not whether the denoiser loaded, so this runs the same with and without
  // dist/dsp. b saves browser mode (lib.mjs): disabled, and the help says why. In DeepFilterNet mode it saves vad.gate,
  // keeps the level marker and persists to a new tab. b is not in a call here, so the mode change only saves.
  const gate = b.getByRole('switch', { name: 'Noise gate' });
  assert.ok(await gate.isDisabled(), 'the noise gate switch is enabled in browser mode');
  assert.match(await b.getByText('Works only with DeepFilterNet noise suppression.').innerText(), /DeepFilterNet/);
  await b.getByLabel('Noise suppression', { exact: true }).selectOption('deepfilter');
  await gate.check();
  assert.deepEqual(await b.evaluate(() => JSON.parse(localStorage.getItem('vesper.audio')).vad), { thresholdDb: -50, gate: true });
  const b3 = await b.context().newPage();
  await b3.goto(`${ORIGIN}/settings/voice`);
  assert.ok(await b3.getByRole('switch', { name: 'Noise gate' }).isChecked(), 'the noise gate did not persist');
  await b3.close();
  await gate.uncheck();
  await b.getByLabel('Noise suppression', { exact: true }).selectOption('browser');
  assert.ok(await gate.isDisabled());
  assert.equal(await b.evaluate(() => JSON.parse(localStorage.getItem('vesper.audio')).vad.gate), false);
  const depth = await b.evaluate(() => history.length);
  await b.getByRole('button', { name: 'Close' }).click();
  await b.waitForURL(`${ORIGIN}/r/${general}`);
  assert.equal(await b.evaluate(() => history.length), depth, 'Close from a direct load added a history entry');
  await b.getByRole('button', { name: 'Settings' }).click();
  await b.getByRole('heading', { name: 'Voice', level: 2 }).waitFor();
  await b.keyboard.press('Escape');
  await b.waitForURL(`${ORIGIN}/r/${general}`);
  console.log('settings persist, close and Esc: ok');

  // Mentions: bob turned notifications on in the welcome steps. A mention while he reads another room shows one
  // notification and puts the count in the title; a message without a mention shows none; with the switch off, none. No
  // page load asks for permission: only turning the switch on does.
  {
    const post = (body) => call(a, 'POST', `/api/rooms/${general}/messages`, { body, reply_to: null, attachments: [] });
    await openRoom(b, 'music');
    const shown = () => b.evaluate(() => window.__shown);
    await post('no mention in this one');
    await post('hey @bob can you look at this');
    await b.waitForFunction(() => window.__shown.length > 0);
    await sleep(500);
    assert.deepEqual(await shown(), [{ title: 'alice mentioned you in general', body: 'hey @bob can you look at this', tag: String(general) }]);
    await b.waitForFunction(() => document.title === '(1) Vesper');
    // A click on the notification closes it and opens the room at the message.
    await b.evaluate(() => window.__notes[0].onclick());
    await b.waitForURL(`${ORIGIN}/r/${general}`);
    await b.getByRole('heading', { level: 1, name: 'general', exact: true }).waitFor();
    await b.locator('article').filter({ hasText: 'hey @bob can you look at this' }).waitFor();
    assert.ok(await b.evaluate(() => window.__notes[0].closed), 'the clicked notification stayed open');
    await b.goto(`${ORIGIN}/settings/notifications`);
    const sw = b.getByRole('switch', { name: 'Notify me when someone mentions me' });
    await sw.waitFor();
    assert.ok(await sw.isChecked(), 'the mention switch is off after the welcome turned it on');
    await b.getByRole('status').filter({ hasText: 'Allowed' }).waitFor();
    await b.getByRole('button', { name: 'Send a test notification' }).waitFor();
    assert.equal(await b.evaluate(() => window.__asked), 0, 'permission asked on load');
    assert.ok(!(await b.getByRole('switch', { name: 'Others joining and leaving' }).isChecked()), 'join and leave sounds should be off by default');
    assert.ok(await b.getByRole('switch', { name: 'Mute and deafen' }).isChecked(), 'mute and deafen sounds should be on by default');
    await sw.uncheck();
    assert.equal(await b.evaluate(() => JSON.parse(localStorage.getItem('vesper.notify')).mentions), false);
    await post('@bob again, with the switch off');
    await sleep(1000);
    assert.deepEqual(await shown(), [], 'a notification with the switch off');
    await sw.check();
    assert.equal(await b.evaluate(() => window.__asked), 1, 'turning the switch on did not ask');
    await b.goto(`${ORIGIN}/r/${general}`);
    await b.waitForFunction(() => document.title === 'Vesper'); // general read at the bottom: the count clears
    console.log('mention notifications and title count: ok');
  }

  // Settings > Account, devices: alice signs in on a second browser, the first lists both and signs the other out, and
  // the second lands on the sign-in page with the session notice.
  const alice2 = await browser('alice-2', []);
  const a2 = alice2.page;
  await a2.goto(`${ORIGIN}/login`);
  await a2.getByLabel('Username').fill('alice');
  await a2.getByLabel('Password', { exact: true }).fill('test-password-1');
  await a2.getByRole('button', { name: 'Sign in' }).click();
  await signedIn(a2);
  await a.goto(`${ORIGIN}/settings/account`);
  const deviceRows = a.getByRole('list', { name: 'Signed-in devices' }).getByRole('listitem');
  await deviceRows.nth(1).waitFor();
  assert.equal(await deviceRows.count(), 2, 'alice should have two devices');
  assert.match(await deviceRows.first().innerText(), /This device/, 'this device is not first');
  const other = deviceRows.filter({ hasNotText: 'This device' });
  assert.match(await other.innerText(), /Last seen just now/);
  await other.getByRole('button', { name: 'Sign out' }).click();
  await a2.waitForURL(/\/login/);
  await a2.getByText('Your session ended. Sign in again.').waitFor();
  await deviceRows.nth(1).waitFor({ state: 'detached' });
  assert.equal(await deviceRows.count(), 1);
  alice.errors.push(...alice2.errors);
  console.log('account devices sign out: ok');

  // Password: a wrong current password says so, empties all three fields and takes focus. A right one changes it: the
  // server drops bob's other sessions (his second browser lands on the sign-in page), the old password no longer signs
  // in and the new one does. 3 sign ins and 2 changes here: the login limit is 5 a minute per ip+user.
  const bob2 = await browser('bob-2', []);
  const bd = bob2.page;
  const bobSignIn = async (password) => {
    await bd.getByLabel('Username').fill('bob');
    await bd.getByLabel('Password', { exact: true }).fill(password);
    await bd.getByRole('button', { name: 'Sign in' }).click();
  };
  await bd.goto(`${ORIGIN}/login`);
  await bobSignIn('test-password-1');
  await signedIn(bd);
  await b.goto(`${ORIGIN}/settings/account`);
  const bobRows = b.getByRole('list', { name: 'Signed-in devices' }).getByRole('listitem');
  await bobRows.nth(1).waitFor();
  const pw = (label) => b.getByLabel(label, { exact: true });
  const fillPw = async (old, next, confirm) => {
    await pw('Current password').fill(old);
    await pw('New password').fill(next);
    await pw('Confirm new password').fill(confirm);
    await b.getByRole('button', { name: 'Change password' }).click();
  };
  const pwFields = () => Promise.all(['Current password', 'New password', 'Confirm new password'].map((l) => pw(l).inputValue()));
  await fillPw('test-password-9', 'test-password-2', 'test-password-2');
  await b.getByText('Your current password is not right.').waitFor();
  assert.deepEqual(await pwFields(), ['', '', ''], 'a password stayed in the form after a failed change');
  assert.equal(await focusedText(b), 'Your current password is not right.', 'focus did not move to the error');
  await fillPw('test-password-1', 'test-password-2', 'test-password-3');
  await b.getByText('The two new passwords are not the same.').waitFor();
  assert.equal(await focusedText(b), 'The two new passwords are not the same.', 'focus did not move to the mismatch');
  await pw('Confirm new password').fill('test-password-2');
  await b.getByRole('button', { name: 'Change password' }).click();
  await b.getByText('Password changed. Your other devices were signed out.').waitFor();
  assert.deepEqual(await pwFields(), ['', '', '']);
  await bd.waitForURL(/\/login/);
  await bd.getByText('Your session ended. Sign in again.').waitFor();
  await bobRows.nth(1).waitFor({ state: 'detached' });
  assert.equal(await bobRows.count(), 1, 'the other device is still listed');
  await bobSignIn('test-password-1');
  await bd.getByText('Wrong username or password.').waitFor();
  await bobSignIn('test-password-2');
  await signedIn(bd);
  bob.errors.push(...bob2.errors);
  await b.getByRole('radio', { name: 'Blue' }).check();
  await b.getByRole('button', { name: 'Save' }).click();
  await b.getByRole('status').filter({ hasText: 'Saved.' }).waitFor();
  assert.equal((await call(b, 'GET', '/api/me')).user.accent, '#6db0f4', 'colour not saved');
  await openRoom(b, 'general');
  console.log('account password change and profile save: ok');

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
  await searchBox.fill('x'.repeat(10_000)); // a long paste is cut to the box's 200 characters
  assert.equal((await searchBox.inputValue()).length, 200, 'the box took more than 200 characters');
  await searchBox.fill('');
  // While the caret is in a from: or in: word the box lists people or rooms: Enter and Tab pick one.
  assert.equal(await searchBox.getAttribute('aria-controls'), null, 'aria-controls set with no list open');
  await searchBox.pressSequentially('from:bo');
  const people = a.getByRole('listbox', { name: 'People' });
  await people.getByRole('option', { name: /bob/ }).waitFor();
  assert.equal(await searchBox.getAttribute('aria-expanded'), 'true');
  assert.equal(await searchBox.getAttribute('aria-controls'), await people.getAttribute('id'), 'aria-controls is not the list');
  await a.keyboard.press('Enter');
  await people.waitFor({ state: 'hidden' });
  await searchBox.pressSequentially('in:gen');
  await a.getByRole('listbox', { name: 'Rooms' }).getByRole('option', { name: 'general' }).waitFor();
  await a.keyboard.press('Tab');
  assert.equal(await searchBox.inputValue(), 'from:bob in:general ', 'the picks did not complete the box');
  assert.ok(await searchBox.evaluate((e) => e === document.activeElement), 'Tab left the box while picking');
  await a.getByText('Add a word to search for.').waitFor();
  await a.keyboard.press('Escape');
  await searchBox.waitFor({ state: 'hidden' });
  console.log('search opens from the keys and the switcher, and picks from: and in: names: ok');

  // A room link opened signed out: sign in returns to that room (music), not to the first one; Back does not loop to
  // sign in. A room id nobody can see gives the plain missing state after sign in.
  const signInAs = async (page, path) => {
    await page.goto(`${ORIGIN}${path}`);
    await page.waitForURL(`${ORIGIN}/login?next=${encodeURIComponent(path)}`);
    await page.getByLabel('Username').fill('alice');
    await page.getByLabel('Password', { exact: true }).fill('test-password-1');
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

  // A room made private while bob's search is open: its hit goes from his list, and the search runs again.
  const den = await call(a, 'POST', '/api/rooms', { name: 'den' });
  await roomLink(b, 'den').waitFor();
  await call(a, 'POST', `/api/rooms/${den.id}/messages`, { body: 'walrus plans in the den', reply_to: null, attachments: [] });
  await searchFor(b, 'walrus', { keys: true });
  await searchHits(b).first().waitFor({ timeout: 5000 });
  await call(a, 'PATCH', `/api/rooms/${den.id}`, { restricted: true });
  await b.getByText('No messages found.').waitFor({ timeout: 5000 });
  assert.equal(await searchHits(b).count(), 0, 'a hit from a room bob lost stayed in his open search');
  await b.keyboard.press('Escape');
  console.log('open search drops a room that became private: ok');

  // The denoiser (worklet, worker, WASM, model) under the server's CSP. A CSP block in the worker is not a page console
  // message: it shows as the fallback reason in the status line. Runs only when dist/dsp/ was built (CI downloads it from
  // the dsp-wasm job). A new account from an invite: alice has used up her sign-in budget (5 a minute) by now.
  const pages = [alice.page, bob.page];
  if (existsSync(new URL('../dist/dsp/vesper_dsp_bg.wasm', import.meta.url))) {
    const dsp = await browser('dana', []);
    const d = dsp.page;
    // With the noise gate on, so its worklet message runs in a real browser too.
    await d.addInitScript(() => localStorage.setItem('vesper.audio', JSON.stringify({ mode: 'deepfilter', vad: { thresholdDb: -50, gate: true } })));
    await signup(d, (await call(a, 'POST', '/api/invites', { role: 'member', max_uses: 1, expires_hours: 1 })).url, 'dana');
    await join(d, 'general');
    const line = d.getByText('Noise suppression:').first();
    let status = '';
    for (let i = 0; i < 80 && (i < 8 || status.includes('loading')); i++) {
      await sleep(250);
      status = await line.innerText();
    }
    assert.equal(status, 'Noise suppression: deepfilter', 'denoiser did not start under the CSP');
    await d.getByRole('link', { name: 'Call details' }).click();
    const diag = d.getByRole('group', { name: 'Call numbers' }).getByText('Noise suppression:');
    assert.equal(await diag.innerText(), 'Noise suppression: deepfilter', 'Diagnostics shows another mode');
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
