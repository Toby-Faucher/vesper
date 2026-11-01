// UI flows for the admin pages, in real Chromium against the release server: alice (owner) creates an invite in the UI,
// copies the link, and bob signs up with it; a revoked link shows the bad-link text; making bob a moderator gives him the
// Admin menu entry and the moderator view; a member who opens /admin sees the not-allowed page; a ban of carol ends her
// session. Invite links are never printed. Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import assert from 'node:assert/strict';
import { ORIGIN, browser, cli, cspViolations, signedIn, signup, sleep, startServer } from './lib.mjs';

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

const stop = await startServer();
try {
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

  // Disk use: three meters; uploads are counted against the default 10 GB quota.
  await openAdmin(a, 'Disk use');
  const uploads = a.getByRole('meter', { name: 'Uploads' });
  assert.equal(await uploads.getAttribute('aria-valuetext'), '0 B of 10 GB');
  assert.equal(await uploads.getAttribute('aria-valuenow'), '0');
  assert.equal(await a.getByRole('meter').count(), 3);
  console.log('disk use: ok');

  // The role change asks first and names bob; Cancel keeps him a member. Then bob is a moderator: the Admin entry, no role
  // or ban controls, no owner sections, and /admin/rooms says it is the owner's.
  await openAdmin(a, 'Members');
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
  console.log('role change confirm, moderator view: ok');

  // A ban asks first and names carol; her open session ends; the row says Banned with Unban.
  await a.getByRole('button', { name: 'Ban carol' }).click();
  const confirmBan = a.getByRole('dialog', { name: 'Ban carol?' });
  await confirmBan.getByText('They are signed out everywhere and cannot sign in.').waitFor();
  await confirmBan.getByRole('button', { name: 'Ban' }).click();
  await c.getByText('Your session ended. Sign in again.').waitFor();
  await a.locator('tr[data-user="carol"]').getByText('Banned').waitFor();
  await a.getByRole('button', { name: 'Unban carol' }).waitFor();
  console.log('ban ends the session: ok');

  for (const p of [a, b, s]) assert.deepEqual(await cspViolations(p), [], 'CSP violations');
  assert.deepEqual([alice, bob, carol, stranger].flatMap((x) => x.errors), [], 'page errors');
  await signedIn(a);
  console.log('e2e ui: PASS');
} finally {
  await stop();
}
