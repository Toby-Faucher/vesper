// UI flows for the admin pages, in real Chromium against the release server: alice (owner) creates an invite in the UI,
// copies the link, and bob signs up with it; a revoked link shows the bad-link text; making bob a moderator gives him the
// Admin menu entry and the moderator view (no Revoke on a moderator invite); a member who opens /admin sees the not-allowed
// page; a ban of carol ends her session; a reset link for bob ends his sessions and sets his new password. Invite links are never printed. Before those, the primitives' keyboard paths
// (primitives.mjs, on the dev-only gallery page). After them: layout shift after a join, reduced motion, and a keyboard-only
// walk with a focus ring on every Tab stop. Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import assert from 'node:assert/strict';
import { ORIGIN, browser, cli, composer, cspViolations, devServer, openRoom, png1x1, searchHits, signedIn, signup, sleep, startServer } from './lib.mjs';
import { checkPrimitives } from './primitives.mjs';

const BAD_LINK = 'This invite link does not work. Ask the person who sent it for a new one.';
const DENIED = 'You do not have access to admin pages.';

async function openAdmin(page, section) {
  await page.getByRole('button', { name: /^Account, / }).click();
  await page.getByRole('menuitem', { name: 'Admin' }).click();
  await page.getByRole('navigation', { name: 'Admin sections' }).getByRole('link', { name: section }).click();
  await page.locator('#main').getByRole('heading', { level: 2, name: section }).waitFor();
}
// Creates an invite from the form and returns the link from the read-only field (kept in memory, never printed).
async function createInvite(page) {
  await page.getByRole('button', { name: 'Create invite' }).click();
  await page.getByText('This link is shown only now.', { exact: false }).waitFor();
  return page.getByLabel('Invite link', { exact: true }).inputValue();
}

// Records the keyframes of every Element.animate call (anime's WAAPI build uses it), for the motion checks.
const recordMotion = () => {
  window.__motion = [];
  window.__targets = []; // what each call animated, for the "does not animate" checks
  const animate = Element.prototype.animate;
  Element.prototype.animate = function (k, o) {
    window.__motion.push(JSON.stringify(k));
    window.__targets.push(this.tagName.toLowerCase());
    return animate.call(this, k, o);
  };
};
const animatedSince = (page) => page.evaluate(() => window.__targets.splice(0));
// The keyframes animated since the last call: the recorded ones plus the CSS animations and transitions running now.
const motionSince = (page) => page.evaluate(() => {
  const all = [...window.__motion, ...document.getAnimations().map((a) => JSON.stringify(a.effect?.getKeyframes?.() ?? []))];
  window.__motion = [];
  return all;
});
const moves = (frames) => frames.filter((f) => /"(transform|translate|scale|rotate)"/.test(f));

async function signIn(page, name) {
  await page.goto(`${ORIGIN}/login`);
  await page.getByLabel('Username').fill(name);
  await page.getByLabel('Password', { exact: true }).fill('test-password-1');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await signedIn(page);
}

const stop = await startServer();
try {
  // The primitives' keyboard paths, on the dev-only gallery page from a Vite dev server.
  const gallery = await browser('gallery');
  await checkPrimitives(async (u) => {
    const p = await gallery.page.context().newPage();
    await p.goto(u);
    return p;
  }, `${await devServer()}/_gallery?primitives`);
  console.log('primitives keyboard paths: ok');

  const alice = await browser('alice');
  const a = alice.page;
  await signup(a, await cli(['create-owner']), 'alice');
  await a.context().grantPermissions(['clipboard-read', 'clipboard-write'], { origin: ORIGIN });

  // Create: one request for two quick clicks, the link is the field's value, Copy puts it on the clipboard, and it is in no
  // storage of the page.
  await openAdmin(a, 'Invites');
  assert.equal(new URL(a.url()).pathname, '/admin/invites');
  const posts = [];
  a.on('request', (r) => r.method() === 'POST' && r.url().endsWith('/api/invites') && posts.push(1));
  await a.route('**/api/invites', async (r) => { if (r.request().method() === 'POST') await new Promise((ok) => setTimeout(ok, 400)); await r.continue(); });
  await a.getByRole('button', { name: 'Create invite' }).click();
  await a.getByRole('button', { name: 'Create invite' }).click({ force: true });
  await a.getByText('This link is shown only now. It works for 1 person until', { exact: false }).waitFor();
  await a.unroute('**/api/invites');
  assert.equal(posts.length, 1, 'a second click on Create invite sent a second request');
  const link = await a.getByLabel('Invite link', { exact: true }).inputValue();
  assert.ok(link.startsWith(`${ORIGIN}/invite#`), 'the field does not hold an invite link');
  await a.getByRole('button', { name: 'Copy link' }).click();
  await a.getByText('Link copied.').waitFor();
  assert.equal(await a.evaluate(() => navigator.clipboard.readText()), link, 'Copy link did not copy the link');
  const code = link.split('#')[1];
  assert.ok(!(await a.evaluate((c) => JSON.stringify({ ...localStorage, ...sessionStorage }).includes(c), code)), 'the invite code is in storage');
  await a.getByRole('row').filter({ hasText: 'Active' }).first().waitFor();
  console.log('create invite, one request, copy, nothing stored: ok');

  const bob = await browser('bob');
  const b = bob.page;
  await signup(b, link, 'bob');
  console.log('signup with the copied link: ok');

  // Revoke asks first, then the row says Revoked and the link no longer works.
  await a.getByRole('button', { name: 'Done' }).click();
  const second = await createInvite(a);
  await a.getByRole('button', { name: 'Done' }).click();
  const deletes = [];
  a.on('request', (r) => r.method() === 'DELETE' && r.url().includes('/api/invites/') && deletes.push(1));
  const row = a.locator('tr[data-invite]').first(); // newest first
  await row.getByRole('button', { name: 'Revoke' }).click();
  const dlg = a.getByRole('dialog', { name: 'Revoke this invite?' });
  await dlg.getByRole('button', { name: 'Cancel' }).click();
  await dlg.waitFor({ state: 'hidden' });
  assert.equal(deletes.length, 0, 'Revoke called the server before the confirm');
  await row.getByRole('button', { name: 'Revoke' }).click();
  await dlg.getByRole('button', { name: 'Revoke' }).click();
  await row.getByText('Revoked').waitFor();
  assert.equal(await row.getByRole('button', { name: 'Revoke' }).count(), 0, 'a revoked invite still offers Revoke');
  assert.equal(deletes.length, 1);
  const stranger = await browser('stranger');
  const s = stranger.page;
  await s.goto(second);
  await s.getByLabel('Username').fill('dave');
  await s.getByLabel('Display name').fill('dave');
  await s.getByLabel('Password', { exact: true }).fill('test-password-1');
  await s.getByRole('button', { name: 'Create account' }).click();
  await s.getByText(BAD_LINK).waitFor();
  console.log('revoke confirm and a revoked link: ok');

  // A member: no Admin entry, and /admin says so with a way back.
  await b.getByRole('button', { name: /^Account, / }).click();
  await b.getByRole('menuitem', { name: 'Log out' }).waitFor();
  assert.equal(await b.getByRole('menuitem', { name: 'Admin' }).count(), 0, 'a member sees the Admin entry');
  await b.keyboard.press('Escape');
  await b.goto(`${ORIGIN}/admin/members`);
  await b.getByText(DENIED).waitFor();
  assert.equal(await b.getByRole('navigation', { name: 'Admin sections' }).count(), 0);
  assert.equal(await b.locator('tr[data-user]').count(), 0, 'a member sees the members table');
  await b.getByRole('button', { name: 'Go to your rooms' }).click();
  await b.waitForURL(/\/r\/\d+$/);
  console.log('member at /admin: not allowed: ok');

  // Popups stay inside the viewport at 1024x640: the Settings tooltip at the bottom of the sidebar (it went under the edge)
  // flips above, the account menu opens upward. The gear turns on hover (a transform transition).
  await b.setViewportSize({ width: 1024, height: 640 });
  const cog = b.getByRole('button', { name: 'Settings', exact: true });
  await cog.hover();
  assert.ok((await b.evaluate(() => document.getAnimations().map((x) => JSON.stringify(x.effect?.getKeyframes?.() ?? [])))).some((f) => f.includes('"transform"')),
    'the Settings gear does not turn on hover');
  await sleep(700); // the tooltip waits 400 ms, then fades and scales in over 160 ms
  const inside = (r) => r.left >= 0 && r.top >= 0 && r.right <= 1024 && r.bottom <= 640;
  const tip = await cog.evaluate((el) => {
    const r = el.getBoundingClientRect(), s = getComputedStyle(el, '::after'), m = new DOMMatrix(s.transform);
    const left = r.left + parseFloat(s.left) + m.e, top = r.top + parseFloat(s.top) + m.f;
    return { left, top, right: left + parseFloat(s.width), bottom: top + parseFloat(s.height), shown: s.opacity === '1' };
  });
  assert.ok(tip.shown && inside(tip), `the Settings tooltip is not inside the viewport: ${JSON.stringify(tip)}`);
  await b.mouse.move(600, 300);
  await b.getByRole('button', { name: /^Account, / }).click();
  const menu = await b.getByRole('menu', { name: 'Account' }).evaluate((el) => el.getBoundingClientRect().toJSON());
  assert.ok(inside(menu), `the account menu is not inside the viewport: ${JSON.stringify(menu)}`);
  await b.keyboard.press('Escape');
  await b.setViewportSize({ width: 1280, height: 720 });
  console.log('popups inside the viewport, gear turns: ok');

  // carol, a member, so the moderator view below has a lower row that must still show no controls.
  const { url: carolLink } = await a.evaluate(async () =>
    (await fetch('/api/invites', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ role: 'member', max_uses: 1, expires_hours: 1 }) })).json());
  const carol = await browser('carol');
  const c = carol.page;
  await signup(c, carolLink, 'carol');

  // Rooms: alice creates music (one request for two clicks), renames it, moves it up by button and by Alt+Up, and restricts
  // it to bob. bob's sidebar follows each change live; carol, a member not listed, never sees it once restricted.
  const order = (p, names) => p.waitForFunction(
    (n) => JSON.stringify([...document.querySelectorAll('nav[aria-label="Rooms"] a[title]')].map((x) => x.title)) === JSON.stringify(n), names);
  await openAdmin(a, 'Rooms');
  await a.getByLabel('Room name').fill('   ');
  await a.getByRole('button', { name: 'Create room' }).click();
  await a.getByRole('alert').getByText('Room names are 1 to 32 characters, with no hidden characters.').waitFor();
  await a.getByLabel('Room name').fill('General');
  await a.getByRole('button', { name: 'Create room' }).click();
  await a.getByRole('alert').getByText('That name is already taken.').waitFor();
  const roomPosts = [];
  a.on('request', (r) => r.method() === 'POST' && r.url().endsWith('/api/rooms') && roomPosts.push(1));
  await a.route('**/api/rooms', async (r) => { if (r.request().method() === 'POST') await new Promise((ok) => setTimeout(ok, 400)); await r.continue(); });
  await a.getByLabel('Room name').fill('music');
  await a.getByRole('button', { name: 'Create room' }).click();
  await a.getByRole('button', { name: 'Create room' }).click({ force: true });
  await a.getByText('Created music.').waitFor();
  await a.unroute('**/api/rooms');
  assert.equal(roomPosts.length, 1, 'a second click on Create room sent a second request');
  await order(b, ['general', 'hangout', 'music']);
  await order(c, ['general', 'hangout', 'music']);

  await a.getByRole('button', { name: 'Rename music' }).click();
  await a.getByLabel('Name for music').fill('not this');
  await a.keyboard.press('Escape');
  assert.equal(await a.evaluate(() => document.activeElement?.getAttribute('aria-label')), 'Rename music', 'Esc did not give focus back to Rename');
  await a.getByRole('button', { name: 'Rename music' }).click();
  await a.getByLabel('Name for music').fill('music room');
  await a.keyboard.press('Enter');
  await a.getByRole('button', { name: 'Rename music room' }).waitFor();
  await order(b, ['general', 'hangout', 'music room']);

  await a.getByRole('button', { name: 'Move music room up' }).click();
  await order(b, ['general', 'music room', 'hangout']);
  // bob's event can land before alice's PUT answers, and a move is ignored while one is in flight: wait for her row.
  await order(a, ['general', 'music room', 'hangout']);
  // Alt+Up from the Move up button: at the top it is disabled, so focus must move to Move down in the same row.
  await a.getByRole('button', { name: 'Move music room up' }).focus();
  await a.keyboard.press('Alt+ArrowUp');
  await order(b, ['music room', 'general', 'hangout']);
  // alice's own row moves once her PUT answers, which can be after bob's event: wait for it, then the focus must be kept.
  await order(a, ['music room', 'general', 'hangout']);
  await a.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Move music room down', null, { timeout: 2000 })
    .catch(() => assert.fail('Alt+Up lost the focus'));
  assert.ok(await a.getByRole('button', { name: 'Move music room up' }).isDisabled(), 'the top room can still move up');
  // A 409 (the list changed elsewhere) says so; nothing moves.
  await a.route('**/api/rooms/order', (r) => r.fulfill({ status: 409, json: { code: 'conflict', message: 'the order must list every room once' } }));
  await a.getByRole('button', { name: 'Move general down' }).click();
  await a.getByRole('alert').getByText('The room list changed. Try again.').waitFor();
  await a.unroute('**/api/rooms/order');
  await order(a, ['music room', 'general', 'hangout']);

  await a.getByRole('switch', { name: 'Restricted: music room' }).check();
  await order(c, ['general', 'hangout']);
  await order(b, ['general', 'hangout']);
  await a.getByRole('button', { name: 'Members of music room' }).click();
  const pick = a.getByRole('dialog', { name: 'Who can see music room?' });
  await pick.getByRole('checkbox', { name: /^bob\b/ }).check();
  await pick.getByRole('button', { name: 'Save' }).click();
  await pick.waitFor({ state: 'hidden' });
  await order(b, ['music room', 'general', 'hangout']);
  await a.waitForFunction(() => document.querySelector('[aria-label="Members of music room"]')?.textContent.replace(/\s+/g, ' ').includes('Members 1'));
  await sleep(500); // a late event would show it to carol here
  await order(c, ['general', 'hangout']);
  console.log('rooms: create, rename, reorder, restrict, live sidebars: ok');

  // Disk use: two meters (the database shows its size only); uploads are counted against the default 10 GB quota.
  await openAdmin(a, 'Disk use');
  const uploads = a.getByRole('meter', { name: 'Uploads' });
  assert.equal(await uploads.getAttribute('aria-valuetext'), '0 B of 10 GB');
  assert.equal(await uploads.getAttribute('aria-valuenow'), '0');
  assert.equal(await a.getByRole('meter').count(), 2);
  await a.locator('#main').getByText('Database').waitFor();
  console.log('disk use: ok');

  // The role change asks first and names bob; Cancel keeps him a member. Then bob is a moderator: the Admin entry, no role
  // or ban controls, no owner sections, and /admin/rooms says it is the owner's.
  // A failed load says so and offers Try again.
  await a.route('**/api/users', (r) => r.abort(), { times: 1 });
  await openAdmin(a, 'Members');
  await a.getByRole('alert').getByText('Could not load the members.', { exact: false }).waitFor();
  await a.getByRole('button', { name: 'Try again' }).click();
  await a.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Members', null, { timeout: 5000 })
    .catch(() => assert.fail('Try again did not move focus to the members table'));
  const roleFor = a.getByLabel('Role for bob');
  await roleFor.selectOption('moderator');
  const confirmRole = a.getByRole('dialog', { name: 'Make bob a moderator?' });
  await confirmRole.getByRole('button', { name: 'Cancel' }).click();
  await confirmRole.waitFor({ state: 'hidden' });
  assert.equal(await roleFor.inputValue(), 'member', 'Cancel left the select on the new role');
  await roleFor.selectOption('moderator');
  await confirmRole.getByRole('button', { name: 'Change role' }).click();
  await confirmRole.waitFor({ state: 'hidden' });
  assert.equal(await roleFor.inputValue(), 'moderator');
  await openAdmin(b, 'Members');
  await b.getByText('Only the owner can change roles and ban people.').waitFor();
  await b.locator('tr[data-user="carol"]').waitFor();
  assert.equal(await b.getByRole('combobox', { name: /^Role for/ }).count(), 0, 'a moderator sees role selects');
  assert.equal(await b.getByRole('button', { name: /^Ban / }).count(), 0, 'a moderator sees Ban');
  assert.equal(await b.getByRole('navigation', { name: 'Admin sections' }).getByRole('link', { name: 'Rooms' }).count(), 0);
  await b.goto(`${ORIGIN}/admin/rooms`);
  await b.getByText('Only the owner can open this page.').waitFor();
  // A moderator revokes member invites only: no Revoke on the owner's moderator invite, Revoke on his own member invite.
  await a.evaluate(() => fetch('/api/invites', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ role: 'moderator', max_uses: 1, expires_hours: 1 }) }));
  await b.goto(`${ORIGIN}/admin/invites`);
  await createInvite(b);
  const active = b.locator('tr[data-invite]').filter({ hasText: 'Active' });
  await active.filter({ hasText: 'Moderator' }).waitFor();
  assert.equal(await active.filter({ hasText: 'Moderator' }).getByRole('button', { name: 'Revoke' }).count(), 0, 'a moderator can revoke a moderator invite');
  await active.filter({ hasText: 'Member' }).getByRole('button', { name: 'Revoke' }).waitFor();
  console.log('role change confirm, moderator view: ok');

  // A ban asks first and names carol; her open session ends; the row says Banned with Unban.
  await a.getByRole('button', { name: 'Ban carol' }).click();
  const confirmBan = a.getByRole('dialog', { name: 'Ban carol?' });
  await confirmBan.getByText('They are signed out everywhere and cannot sign in.').waitFor();
  await confirmBan.getByRole('button', { name: 'Ban' }).click();
  await c.getByText('Your session ended. Sign in again.').waitFor();
  await a.locator('tr[data-user="carol"]').getByText('Banned').waitFor();
  const banned = await a.locator('tr[data-user="carol"]').getByText('Banned').evaluate((e) => [getComputedStyle(e).color, getComputedStyle(e.closest('tr').querySelector('button span[title]')).color]);
  assert.notEqual(banned[0], banned[1], 'Banned is in the primary text colour');
  await a.getByRole('button', { name: 'Unban carol' }).waitFor();
  console.log('ban ends the session: ok');

  // Layout shift on bob's page, from after his join: opening a room, a message arriving, an image loading, hovering the
  // messages and the typing line. Every shift counts, also those right after his click (CLS drops those, but a room open
  // is a click and the brief measures it).
  await openRoom(a, 'general');
  await openRoom(b, 'general');
  await b.getByRole('button', { name: 'Join voice' }).click();
  await b.getByRole('button', { name: 'Leave' }).waitFor();
  await sleep(500);
  await b.evaluate(() => {
    window.__shift = [];
    new PerformanceObserver((l) => window.__shift.push(...l.getEntries().map((e) => ({ v: e.value, input: e.hadRecentInput })))).observe({ type: 'layout-shift' });
  });
  await openRoom(b, 'hangout');
  await openRoom(b, 'general');
  await sleep(700);
  const general = Number(new URL(b.url()).pathname.split('/').pop());
  const post = (body, attachments = []) => a.evaluate(([room, body, attachments]) => fetch(`/api/rooms/${room}/messages`, {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ body, reply_to: null, attachments }),
  }), [general, body, attachments]);
  await post('a message arriving');
  await b.getByText('a message arriving', { exact: true }).waitFor();
  const img = await a.evaluate(async ([room, bytes]) => (await (await fetch(`/api/rooms/${room}/uploads?name=dot.png`, { method: 'POST', body: new Uint8Array(bytes) })).json()).attachment.id, [general, [...png1x1()]]);
  await post('an image', [img]);
  await b.getByRole('button', { name: 'Open image dot.png' }).waitFor();
  await b.waitForFunction(() => [...document.querySelectorAll('img')].some((i) => i.alt === 'dot.png' && i.complete));
  await sleep(500);
  for (const t of ['a message arriving', 'an image']) { await b.getByText(t, { exact: true }).hover(); await sleep(300); }
  await b.mouse.move(5, 5);
  await composer(a).pressSequentially('typing');
  await b.getByText('alice is typing').waitFor();
  await composer(a).fill('');
  await sleep(500);
  const shifts = await b.evaluate(() => window.__shift);
  const cls = shifts.reduce((n, x) => n + x.v, 0);
  console.log(`layout shift after join: ${cls.toFixed(4)} (${shifts.length} entries, ${shifts.filter((x) => x.input).length} after input)`);
  assert.ok(cls < 0.02, `layout shift ${cls} is 0.02 or more`);
  await b.getByRole('button', { name: 'Leave' }).click();
  console.log('layout shift: ok');

  // Reduced motion: opening a dialog, the call bar, a jump, a new message, a room switch (the sidebar glide too) and a
  // settings section move nothing (opacity only).
  const calm = await browser('calm');
  const r = calm.page;
  await r.emulateMedia({ reducedMotion: 'reduce' });
  await r.addInitScript(recordMotion);
  await signIn(r, 'bob');
  await openRoom(r, 'general');
  await motionSince(r);
  await r.getByRole('button', { name: 'Join voice' }).click();
  await r.getByRole('button', { name: 'Leave' }).waitFor();
  const barMotion = await motionSince(r);
  await r.locator('main').focus();
  await r.keyboard.press('Shift+Slash');
  await r.getByRole('dialog', { name: 'Keyboard shortcuts' }).waitFor();
  const dialogMotion = await motionSince(r);
  await r.keyboard.press('Escape');
  await r.getByRole('dialog', { name: 'Keyboard shortcuts' }).waitFor({ state: 'hidden' });
  await r.getByRole('button', { name: 'Search', exact: true }).click();
  await r.getByLabel('Search messages').fill('arriving');
  await searchHits(r).first().click();
  await r.getByRole('dialog', { name: 'Search' }).waitFor({ state: 'hidden' });
  const jumpMotion = await motionSince(r);
  await r.getByRole('button', { name: 'Settings', exact: true }).hover();
  jumpMotion.push(...(await motionSince(r)));
  await composer(r).fill('calm hello');
  await composer(r).press('Enter');
  await r.locator('article').filter({ hasText: 'calm hello' }).waitFor();
  await openRoom(r, 'hangout');
  await openRoom(r, 'general');
  await r.keyboard.press('Control+Backslash');
  await r.locator('#sidebar').waitFor({ state: 'hidden' });
  await r.keyboard.press('Control+Backslash');
  await r.locator('#sidebar').waitFor();
  await r.getByRole('button', { name: 'Settings', exact: true }).click();
  await r.getByRole('navigation', { name: 'Settings sections' }).getByRole('link', { name: 'Appearance' }).click();
  await sleep(300);
  const chatMotion = await motionSince(r);
  assert.ok(dialogMotion.length && barMotion.length && chatMotion.length, 'no motion recorded: the hook missed the animations');
  assert.deepEqual(moves([...dialogMotion, ...barMotion, ...jumpMotion, ...chatMotion]), [], 'something moves under reduced motion');
  await r.keyboard.press('Escape');
  await r.getByRole('button', { name: 'Leave' }).click();
  console.log('reduced motion: ok');

  // Keyboard only after the first page load: sign in, Ctrl+K to a room, Join voice by Tab and Enter, Ctrl+Shift+M, send,
  // Up edits, Esc cancels, Ctrl+Shift+F and Enter jumps, Ctrl+K Settings and Esc back. Every Tab stop shows an outline.
  const keys = await browser('keys');
  const k = keys.page;
  await k.addInitScript(recordMotion);
  // The socket goes through Playwright, so the walk can drop it: while `down`, a new socket opens but gets no hello.
  let sock = null;
  let down = false;
  await k.routeWebSocket(/\/ws$/, (ws) => { sock = ws; if (!down) ws.connectToServer(); });
  await k.goto(`${ORIGIN}/login`);
  await k.getByLabel('Username').waitFor();
  const tabTo = async (want) => {
    for (let i = 0; i < 100; i++) {
      await k.keyboard.press('Tab');
      await sleep(150); // a border ring fades in over --dur-1
      const at = await k.evaluate(() => {
        const e = document.activeElement;
        if (!e || e === document.body) return null;
        const cs = getComputedStyle(e);
        const name = (e.getAttribute('aria-label') || e.labels?.[0]?.innerText || e.innerText || '').trim().split('\n')[0];
        // A ring is an outline, or (the composer) a parent's border in the focus colour.
        const probe = document.body.appendChild(document.createElement('i'));
        probe.style.color = 'var(--focus)';
        const focus = getComputedStyle(probe).color;
        probe.remove();
        const parents = [e.parentElement, e.parentElement?.parentElement, e.parentElement?.parentElement?.parentElement].filter(Boolean); // the composer's box is three up
        return { name, ring: (cs.outlineStyle !== 'none' && cs.outlineWidth !== '0px') || parents.some((p) => getComputedStyle(p).borderTopColor === focus) };
      });
      if (!at) continue;
      assert.ok(at.ring, `no focus outline on ${JSON.stringify(at.name)}`);
      if (at.name === want) return;
    }
    assert.fail(`Tab never reached ${want}`);
  };
  const focusedName = () => k.evaluate(() => document.activeElement?.getAttribute('aria-label'));
  await tabTo('Username');
  await k.keyboard.type('alice');
  await tabTo('Password');
  await k.keyboard.type('test-password-1');
  await k.keyboard.press('Enter');
  await signedIn(k);
  // The skip link is the first Tab stop from the top and Enter moves focus to main.
  await sleep(300); // the home view settles its own focus first
  await k.evaluate(() => { const b = document.body; b.tabIndex = -1; b.focus(); b.removeAttribute('tabindex'); });
  await k.keyboard.press('Tab');
  const first = await k.evaluate(() => document.activeElement?.outerHTML.slice(0, 120));
  assert.match(first, />Skip to main content<\/a>$/, `the skip link is not the first Tab stop: ${first}`);
  await k.keyboard.press('Enter');
  assert.equal(await k.evaluate(() => document.activeElement?.id), 'main', 'the skip link did not move focus to main');
  await k.keyboard.press('Control+k');
  await k.getByRole('combobox', { name: 'Switch to' }).waitFor();
  await k.keyboard.type('general');
  await animatedSince(k);
  await k.keyboard.press('Enter');
  await k.getByRole('heading', { level: 1, name: 'general', exact: true }).waitFor();
  await k.locator('article').first().waitFor();
  await sleep(300);
  assert.deepEqual((await animatedSince(k)).filter((t) => t === 'article'), [], 'the history of a room animated on open');
  await tabTo('Join voice');
  // Offline: Join voice keeps focus, says why (aria-describedby; the tip shows on focus) and Enter does nothing.
  const join = k.getByRole('button', { name: 'Join voice' });
  down = true;
  sock.close();
  await k.waitForFunction(() => document.activeElement?.getAttribute('aria-disabled') === 'true', null, { timeout: 5000 });
  assert.equal(await join.evaluate((e) => e === document.activeElement), true, 'Join voice lost focus when the connection dropped');
  assert.equal(await join.evaluate((e) => document.getElementById(e.getAttribute('aria-describedby'))?.textContent), 'You are offline. This works again once the connection is back.');
  await k.keyboard.press('Enter');
  await sleep(500);
  assert.equal(await k.getByRole('button', { name: 'Leave' }).count(), 0, 'Join voice worked while offline');
  down = false;
  sock.close(); // the socket that got no hello: the next one reaches the server
  await k.waitForFunction(() => document.activeElement?.textContent?.trim() === 'Join voice' && !document.activeElement.hasAttribute('aria-disabled'), null, { timeout: 15000 });
  console.log('offline Join voice: ok');
  await k.keyboard.press('Enter');
  await k.getByRole('button', { name: 'Leave' }).waitFor();
  assert.ok(moves(await motionSince(k)).length > 0, 'the call bar rose without moving: the reduced motion check proves nothing');
  await k.keyboard.press('Control+Shift+M');
  await k.getByRole('button', { name: 'Unmute' }).waitFor();
  await tabTo('Message');
  await sleep(200);
  await animatedSince(k);
  await k.keyboard.type('kiwi from the keyboard');
  assert.deepEqual(await animatedSince(k), [], 'typing in the composer animated something');
  // While the new message rises, scroll events must still read the list as at the bottom (no Jump to latest pill).
  await k.evaluate(() => {
    const list = document.querySelector('[data-timeline]');
    const end = performance.now() + 1500;
    window.__jumped = false;
    const tick = () => {
      list.dispatchEvent(new Event('scroll'));
      window.__jumped ||= !!document.querySelector('[aria-label="Jump to latest"]');
      if (performance.now() < end) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  await k.keyboard.press('Enter');
  await k.locator('article').filter({ hasText: 'kiwi from the keyboard' }).waitFor();
  await sleep(1600);
  assert.equal(await k.evaluate(() => window.__jumped), false, 'a rising message made the list leave the bottom (Jump to latest showed)');
  assert.ok(moves(await motionSince(k)).length > 0, 'a new message did not rise in: the reduced motion check proves nothing');
  await k.keyboard.press('ArrowUp');
  const edit = k.getByRole('textbox', { name: 'Edit message' });
  await edit.waitFor();
  assert.equal(await edit.inputValue(), 'kiwi from the keyboard');
  await k.keyboard.press('Escape');
  await edit.waitFor({ state: 'detached' });
  assert.equal(await focusedName(), 'Message', 'Esc on the edit did not give focus back to the composer');
  await k.keyboard.press('Control+Shift+F');
  await k.getByLabel('Search messages').waitFor();
  await k.keyboard.type('kiwi');
  await searchHits(k).first().waitFor();
  await k.keyboard.press('Enter');
  await k.getByRole('dialog', { name: 'Search' }).waitFor({ state: 'hidden' });
  await k.waitForURL(/\/r\/\d+/);
  await k.keyboard.press('Control+k');
  await k.keyboard.type('Settings');
  await k.getByRole('option', { selected: true, name: /Settings: Voice/ }).waitFor();
  await k.keyboard.press('Enter');
  await k.locator('#main').getByRole('heading', { level: 2, name: 'Voice' }).waitFor();
  await k.keyboard.press('Escape');
  await k.getByRole('heading', { level: 1, name: 'general', exact: true }).waitFor();
  await k.getByRole('button', { name: 'Leave' }).waitFor(); // still in the call
  console.log('keyboard only: ok');

  // The message highlight fades (paint only: background, text colour, opacity) and never moves the row.
  const lastRow = k.locator('article').last();
  const box = () => lastRow.evaluate((e) => [e.offsetTop, e.offsetLeft, e.offsetWidth, e.offsetHeight].join()); // layout, not scroll
  const still = await box();
  await lastRow.hover();
  const fades = await lastRow.evaluate((e) => e.getAnimations({ subtree: true }).map((a) => a.transitionProperty ?? 'script'));
  assert.ok(fades.includes('background-color'), `the hover highlight does not fade: ${fades}`);
  assert.deepEqual(fades.filter((x) => !['background-color', 'color', 'opacity'].includes(x)), [], 'the hover highlight animates more than paint');
  assert.equal(await box(), still, 'the hover highlight moved the row');
  // The sidebar slides out (inert at once, the main pane following by transform) and back; focus stays where it was.
  await composer(k).focus();
  await k.keyboard.press('Control+Backslash');
  const mid = await k.evaluate(() => {
    const side = document.getElementById('sidebar');
    const slides = (id) => document.getElementById(id).getAnimations().some((a) => JSON.stringify(a.effect.getKeyframes()).includes('translateX'));
    return { inert: side.inert, drawn: side.checkVisibility(), slides: [slides('sidebar'), slides('main')] };
  });
  assert.deepEqual(mid, { inert: true, drawn: true, slides: [true, true] }, 'the sidebar did not slide out');
  await k.locator('#sidebar').waitFor({ state: 'hidden' });
  assert.equal(await focusedName(), 'Message', 'hiding the sidebar moved focus out of the composer');
  // The saved hidden state at boot: no slide, nothing animates the shell.
  await k.reload();
  await k.getByRole('button', { name: 'Show sidebar' }).waitFor();
  await sleep(500);
  assert.deepEqual((await animatedSince(k)).filter((t) => t === 'main'), [], 'the saved sidebar state slid at boot');
  assert.equal(await k.locator('#sidebar').evaluate((e) => e.hidden && e.getAnimations().length), 0, 'the hidden sidebar animated at boot');
  await k.getByRole('button', { name: 'Show sidebar' }).click();
  await sleep(50);
  assert.ok((await animatedSince(k)).includes('main'), 'the sidebar did not slide back in');
  assert.equal(await focusedName(), 'Hide sidebar', 'focus did not move to the sidebar toggle');
  console.log('message highlight and sidebar slide: ok');

  // Reset link. Moderator bob gets it on members (carol) only, not on the owner. Alice makes one for bob: the dialog names
  // him, the link shows once and takes focus. Bob sets a new password on /reset in another browser: the code leaves the
  // address bar, his open session lands on sign in, the old password fails, the new one works and tells him once who
  // reset it, the owner's table shows the reset as used, and the link fails twice. Alice must type her own password.
  await openAdmin(b, 'Members');
  await b.getByRole('button', { name: 'Create reset link for carol' }).waitFor();
  assert.equal(await b.getByRole('button', { name: 'Create reset link for alice' }).count(), 0, 'a moderator can reset the owner');
  await openAdmin(a, 'Members');
  await a.getByRole('button', { name: 'Create reset link for bob' }).click();
  const confirmReset = a.getByRole('dialog', { name: 'Create a reset link for bob?' });
  await confirmReset.getByLabel('Your password', { exact: true }).fill('test-password-9');
  await confirmReset.getByRole('button', { name: 'Create link' }).click();
  await confirmReset.getByRole('alert').getByText('Your password is not right.').waitFor();
  assert.equal(await confirmReset.getByLabel('Your password', { exact: true }).inputValue(), '', 'the password stayed after a failed try');
  await confirmReset.getByLabel('Your password', { exact: true }).fill('test-password-1');
  await confirmReset.getByRole('button', { name: 'Create link' }).click();
  await confirmReset.waitFor({ state: 'hidden' });
  const resetBox = a.getByRole('textbox', { name: 'Reset link for bob', exact: true });
  const resetLink = await resetBox.inputValue();
  assert.match(resetLink, new RegExp(`^${ORIGIN}/reset#[0-9a-f]{64}$`), 'not a reset link');
  assert.ok(await resetBox.evaluate((e) => e === document.activeElement), 'focus did not move to the new link');
  const bobNew = await browser('bob-new');
  const bn = bobNew.page;
  await bn.goto(resetLink);
  await bn.getByRole('heading', { name: 'Set a new password' }).waitFor();
  assert.equal(bn.url(), `${ORIGIN}/reset`, 'reset code stayed in the address bar');
  await bn.getByLabel('New password', { exact: true }).fill('test-password-4');
  await bn.getByRole('button', { name: 'Set password' }).click();
  await bn.getByRole('heading', { name: 'Your password is set' }).waitFor();
  await b.waitForURL(/\/login/);
  await b.getByText('Your session ended. Sign in again.').waitFor();
  await bn.getByRole('button', { name: 'Sign in' }).click();
  await bn.getByText('Forgot your password? Ask the owner for a reset link.').waitFor();
  const bobSignIn = async (password) => {
    await bn.getByLabel('Username').fill('bob');
    await bn.getByLabel('Password', { exact: true }).fill(password);
    await bn.getByRole('button', { name: 'Sign in' }).click();
  };
  await bobSignIn('test-password-1');
  await bn.getByText('Wrong username or password.').waitFor();
  await bobSignIn('test-password-4');
  // Told once who reset it, then the app; the owner's table says it was used.
  await bn.getByText('Your password was reset by alice on', { exact: false }).waitFor();
  await bn.getByRole('button', { name: 'Continue' }).click();
  await signedIn(bn);
  await openAdmin(a, 'Invites');
  await openAdmin(a, 'Members');
  await a.locator('tr[data-user="bob"]').getByText(/^Password reset by alice on .+ \(used\)$/).waitFor();
  await bn.goto(resetLink);
  await bn.getByLabel('New password', { exact: true }).fill('test-password-5');
  await bn.getByRole('button', { name: 'Set password' }).click();
  await bn.getByRole('alert').getByText('This reset link does not work.', { exact: false }).waitFor();
  await bn.goto(`${ORIGIN}/`);
  await signedIn(bn); // the failed try ended nothing
  console.log('reset link: moderator scope, owner makes it, bob resets, old password and session dead, single use: ok');

  for (const p of [a, b, s, r, k, bn]) assert.deepEqual(await cspViolations(p), [], 'CSP violations');
  assert.deepEqual([gallery, alice, bob, carol, stranger, calm, keys, bobNew].flatMap((x) => x.errors), [], 'page errors');
  await signedIn(a);
  console.log('e2e ui: PASS');
} finally {
  await stop();
}
