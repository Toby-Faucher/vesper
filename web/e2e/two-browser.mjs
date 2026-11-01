// Two-browser voice and chat test. Starts the release server on a throwaway data dir, makes the owner invite with the CLI, signs
// alice up, has her invite bob through the API, and joins both into `general` with fake microphones. Checks that audio
// flows both ways (bob to alice only as rising packets), that bob hears alice, and that each sees the other in the people list.
// Then chat: Markdown and raw HTML shown safely, a reply, an edit, a PNG upload shown inline, the upload tray, drop, image
// boxes and the viewer, and the search dialog, which never returns a message from a room bob cannot see. Stops the server.
// Needs ../target/release/vesper-server and dist/ (npm run build). Run: npm run e2e
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { browser, cli, png1x1, composer, cspViolations, inVoice, messageMenu, openRoom, roomId, roomLink, searchFor, searchHits, signup, sleep, startServer } from './lib.mjs';

const speech = fileURLToPath(new URL('fixtures/speech.wav', import.meta.url));

async function join(page) {
  await openRoom(page, 'general');
  await page.getByRole('button', { name: 'Join' }).click();
  await page.getByRole('button', { name: 'Leave' }).waitFor();
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

// Sends a message through the API, waiting out a 429: the send limit is a burst of 10, then one a second.
async function post(page, room, body) {
  for (;;) {
    try {
      return await call(page, 'POST', `/api/rooms/${room}/messages`, { body, reply_to: null, attachments: [] });
    } catch (e) {
      if (!String(e).includes(': 429')) throw e;
      await sleep(500);
    }
  }
}

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
  // comes from an analyser on the received stream (Playback.speaking). Bob's own row can ring from his beep, so count alice's.
  let rings = 0;
  let last = first;
  const N = 20;
  for (let i = 0; i < N; i++) {
    await sleep(250);
    last = await inbound(bob.page);
    rings += await bob.page.locator('li.speaking[data-speaking]', { hasText: 'alice' }).count();
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

  // presence_seen_by_both: each page lists the other under general in the sidebar.
  await inVoice(alice.page, 'bob').waitFor({ timeout: 5000 });
  await inVoice(bob.page, 'alice').waitFor({ timeout: 5000 });
  console.log('presence_seen_by_both: ok');

  // welcome_keeps_the_call: Settings > Account > "Run the welcome steps again" opens in the Shell, so the call stays up
  // through the steps and after "Open Vesper".
  const stillInCall = async (when) => {
    const before = await inbound(bob.page);
    await sleep(1000);
    const after = await inbound(bob.page);
    assert.match(after?.ice ?? '', /^(connected|completed)$/, `ICE not connected ${when}`);
    assert.ok(after.packets > before.packets, `bob's audio stopped ${when}`);
    assert.ok(await bob.page.getByRole('button', { name: 'Leave' }).isVisible(), `call bar gone ${when}`);
    await inVoice(alice.page, 'bob').waitFor({ timeout: 2000 });
  };
  await bob.page.getByRole('button', { name: 'Settings', exact: true }).click();
  await bob.page.getByRole('link', { name: 'Account', exact: true }).click();
  await bob.page.getByRole('link', { name: 'Run the welcome steps again' }).click();
  await bob.page.getByRole('heading', { name: 'How others see you' }).waitFor();
  await stillInCall('on the welcome steps');
  await bob.page.getByRole('button', { name: 'Next' }).click();
  await bob.page.getByText('Step 2 of 3').waitFor();
  await bob.page.getByRole('button', { name: 'Next' }).click();
  await bob.page.getByRole('button', { name: 'Open Vesper' }).click();
  await bob.page.waitForURL(/\/r\/\d+$/);
  await stillInCall('after the welcome steps');
  console.log('welcome_keeps_the_call: ok');

  // chat_roundtrip: Markdown renders, raw HTML stays text, the link is safe, a reply and an edit arrive live.
  await composer(alice.page).fill('hello **bob** <b>x</b> https://example.test/a');
  await composer(alice.page).press('Enter');
  const got = boldBob(bob.page);
  await got.waitFor({ timeout: 5000 });
  // innerText, not textContent: what is drawn. A row just added may not be painted yet (content-visibility), so poll.
  await bob.page.waitForFunction((el) => el.innerText.includes('<b>x</b>'), await got.elementHandle(), { timeout: 5000 }).catch(() => assert.fail('raw HTML was not shown as text'));
  assert.equal(await got.locator('b').count(), 0, 'raw HTML became an element');
  // The screen reader hears the rendered text, not the Markdown.
  await bob.page.waitForFunction(() => document.querySelector('p.sr-only[aria-live="polite"]')?.textContent === 'alice: hello bob <b>x</b> https://example.test/a', null, { timeout: 5000 });
  // The same text twice ("ok", "ok") is read twice: every announcement changes the live region.
  await alice.page.evaluate(() => {
    window.__said = 0;
    const region = document.querySelector('p.sr-only[aria-live="polite"]');
    new MutationObserver((rs) => (window.__said += rs.filter((x) => x.addedNodes.length || x.type === 'characterData').length)).observe(region, { childList: true, characterData: true, subtree: true });
  });
  for (let i = 0; i < 2; i++) await post(bob.page, roomId(bob.page), 'ok');
  await alice.page.waitForFunction(() => window.__said >= 2, null, { timeout: 5000 }).catch(() => assert.fail('the second "ok" was not announced'));
  // The name is not checked: alice's page loaded its people before bob signed up, so it says "Someone" (a known gap).
  assert.match(await alice.page.locator('p.sr-only[aria-live="polite"]').textContent(), /: ok$/);
  const link = got.getByRole('link', { name: 'https://example.test/a' });
  assert.equal(await link.getAttribute('href'), 'https://example.test/a');
  assert.equal(await link.getAttribute('rel'), 'noopener noreferrer nofollow');
  assert.equal(await link.getAttribute('target'), '_blank');
  await got.getByRole('button', { name: 'Reply', exact: true }).click();
  await composer(bob.page).fill('hi alice');
  await composer(bob.page).press('Enter');
  await alice.page.locator('article', { hasText: 'Reply to alice' }).filter({ hasText: 'hi alice' }).waitFor({ timeout: 5000 });
  // Edit (in the More menu) gets focus, Escape cancels it and gives focus back to More, Enter saves.
  await messageMenu(boldBob(alice.page), 'Edit');
  const editBox = alice.page.getByLabel('Edit message');
  assert.ok(await editBox.evaluate((el) => el === document.activeElement), 'the edit box did not get focus');
  await editBox.press('Escape');
  await editBox.waitFor({ state: 'detached', timeout: 2000 });
  const more = boldBob(alice.page).getByRole('button', { name: 'More', exact: true });
  assert.ok(await more.evaluate((el) => el === document.activeElement), 'focus did not go back to More after Escape');
  await messageMenu(boldBob(alice.page), 'Edit');
  await editBox.fill('hello **bob** edited');
  await editBox.press('Enter');
  await boldBob(bob.page).filter({ hasText: 'bob edited' }).filter({ hasText: '(edited)' }).waitFor({ timeout: 5000 });
  console.log('chat_roundtrip: ok');

  // The toolbar is one Tab stop per row (roving): Left and Right move along it, Up and Down go to the next or previous
  // row, Esc goes back to the composer. Each action is described by whose message it is.
  const row = boldBob(alice.page);
  const stops = () => row.getByRole('toolbar').getByRole('button').evaluateAll((bs) => bs.map((b) => b.tabIndex));
  const focused = () => alice.page.evaluate(() => document.activeElement?.getAttribute('aria-label') ?? document.activeElement?.tagName);
  await row.getByRole('button', { name: 'More', exact: true }).focus();
  assert.deepEqual(await stops(), [-1, -1, -1, 0], 'the toolbar is not one Tab stop on More');
  await alice.page.keyboard.press('ArrowRight');
  assert.equal(await focused(), 'Add reaction', 'Right did not wrap to the first action');
  await alice.page.keyboard.press('ArrowRight');
  assert.equal(await focused(), 'Reply', 'Right did not move to Reply');
  assert.deepEqual(await stops(), [-1, 0, -1, -1], 'the Tab stop did not move with the focus');
  assert.match(await alice.page.evaluate(() => document.getElementById(document.activeElement.getAttribute('aria-describedby'))?.textContent), /^alice, \d/, 'Reply is not described by its author and time');
  await alice.page.keyboard.press('ArrowDown');
  assert.ok(await row.evaluate((el) => document.activeElement.tagName === 'ARTICLE' && document.activeElement !== el), 'Down did not go to the next row');
  await alice.page.keyboard.press('ArrowUp');
  assert.ok(await row.evaluate((el) => document.activeElement === el), 'Up did not come back to the row');
  await alice.page.keyboard.press('Escape');
  await alice.page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Message', null, { timeout: 2000 }).catch(() => assert.fail('Esc in a row did not go to the composer'));
  console.log('roving_toolbar: ok');

  // composer: bob's typing shows for alice; an Enter during IME composition does not send; @ offers people and Enter
  // inserts the username; Esc cancels a reply; Up in an empty box edits your last message and Esc gives focus back.
  const aBox = composer(alice.page);
  const bBox = composer(bob.page);
  await bBox.fill('');
  await sleep(3000); // bob's last typing event (for "hi alice") may be under 3 s old: the client sends one per 3 s
  await bBox.pressSequentially('hey @al');
  await alice.page.getByRole('status').filter({ hasText: 'bob is typing' }).waitFor({ timeout: 5000 });
  const option = bob.page.getByRole('option', { name: /alice/ });
  await option.waitFor({ timeout: 2000 });
  assert.equal(await bBox.getAttribute('aria-activedescendant'), await option.getAttribute('id'), 'the first person is not active');
  await bBox.press('Enter');
  assert.equal(await bBox.inputValue(), 'hey @alice ', 'Enter did not insert the username');
  await bBox.evaluate((el) => el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true, cancelable: true })));
  await sleep(300);
  assert.equal(await bBox.inputValue(), 'hey @alice ', 'an Enter during IME composition sent the message');
  await bBox.fill('');
  await got.getByRole('button', { name: 'Reply', exact: true }).click();
  await bob.page.getByRole('button', { name: 'Cancel reply' }).waitFor();
  await bBox.press('Escape');
  await bob.page.getByRole('button', { name: 'Cancel reply' }).waitFor({ state: 'detached', timeout: 2000 });
  await aBox.fill('');
  await aBox.press('ArrowUp');
  assert.equal(await editBox.inputValue(), 'hello **bob** edited', 'Up did not edit the last own message');
  await editBox.press('Escape');
  await editBox.waitFor({ state: 'detached', timeout: 2000 });
  assert.ok(await aBox.evaluate((el) => el === document.activeElement), 'focus did not go back to the composer after Esc');
  // markdown_full: every construct of the subset renders, the code block is highlighted by the lazy chunk and copies, the
  // composer styles markdown while typing, and a reply to the message quotes plain text, not the syntax.
  const mdBox = composer(alice.page);
  await mdBox.fill('**x** `y` # no');
  const mirror = alice.page.locator('.mirror');
  assert.equal(await mirror.locator('span.b').textContent(), 'x', 'the composer did not style bold');
  assert.equal(await mirror.locator('span.code').textContent(), 'y', 'the composer did not style code');
  assert.equal(await mirror.textContent(), (await mdBox.inputValue()) + ' ', 'the preview layer does not match the textarea');
  assert.equal(await mdBox.evaluate((el) => getComputedStyle(el).color), 'rgba(0, 0, 0, 0)', 'the textarea text is not transparent');
  // Long wrapped text with styled runs: the layer wraps on the same lines and follows the textarea's scroll.
  await mdBox.fill('a **long** line with `code` and ~~struck~~ words '.repeat(60));
  const fit = await mdBox.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event('scroll'));
    const m = el.parentElement.querySelector('.mirror');
    return { ta: el.scrollHeight, m: m.scrollHeight, top: el.scrollTop, mtop: m.scrollTop };
  });
  assert.ok(fit.ta > 200 && fit.ta === fit.m && fit.top > 0 && fit.top === fit.mtop, `the layer does not line up: ${JSON.stringify(fit)}`);
  const md = ['# Plan', '## Details', 'Some **strong** and *soft* and ~~gone~~ with `tick`.', '> quoted line', '- one', '- two',
    '1. first', '2. second', '---', '```rust', 'fn main() { let x = 1; }', '```'].join('\n');
  await mdBox.fill(md);
  await mdBox.press('Enter');
  const full = bob.page.locator('article').filter({ has: bob.page.getByRole('heading', { name: 'Plan' }) });
  await full.waitFor({ timeout: 5000 });
  assert.equal(await full.getByRole('heading', { name: 'Details' }).getAttribute('aria-level'), '5');
  for (const [sel, text] of [['strong', 'strong'], ['em', 'soft'], ['s', 'gone'], ['code', 'tick'], ['blockquote', 'quoted line'],
    ['ul li', 'one'], ['ol li', 'second']])
    assert.equal(await full.locator(sel, { hasText: text }).count(), 1, `${sel} "${text}" did not render`);
  assert.equal(await full.locator('hr').count(), 1, 'the rule did not render');
  assert.equal(await full.locator('ol').getAttribute('start'), '1');
  assert.ok(!(await full.innerText()).includes('**'), 'raw markers are shown');
  await full.locator('pre span.kw', { hasText: 'fn' }).waitFor({ timeout: 5000 }).catch(() => assert.fail('the code block was not highlighted'));
  assert.equal(await full.locator('pre').textContent(), 'fn main() { let x = 1; }');
  await full.getByRole('button', { name: 'Copy code' }).click();
  await full.getByRole('button', { name: 'Copied' }).waitFor({ timeout: 2000 });
  await full.hover();
  await full.getByRole('button', { name: 'Reply', exact: true }).click();
  const bar = bob.page.locator('form .bar');
  await bar.waitFor();
  const barText = await bar.innerText();
  assert.match(barText, /Plan Details Some strong and soft/, `the reply bar: ${barText}`);
  assert.ok(!/[#*`~]|^>/m.test(barText), `the reply bar shows markdown: ${barText}`);
  await composer(bob.page).fill('noted');
  await composer(bob.page).press('Enter');
  const quote = alice.page.locator('article', { hasText: 'noted' }).locator('button.quote');
  await quote.waitFor({ timeout: 5000 });
  const quoteText = await quote.innerText();
  assert.match(quoteText, /Reply to alice: Plan Details Some strong and soft/, `the reply quote: ${quoteText}`);
  assert.ok(!/[#*`~]/.test(quoteText), `the reply quote shows markdown: ${quoteText}`);
  console.log('markdown_full: ok');
  // A draft belongs to its room: it is not shown in another room and is there again on the way back.
  await call(alice.page, 'POST', '/api/rooms', { name: 'drafts' });
  await aBox.fill('only for general');
  await openRoom(alice.page, 'drafts');
  assert.equal(await aBox.inputValue(), '', 'the general draft showed in another room');
  await openRoom(alice.page, 'general');
  assert.equal(await aBox.inputValue(), 'only for general', 'the draft was lost');
  await aBox.fill('');
  // Pins: pin from the toolbar, find it in the header's list, unpin it there, then the empty state.
  await boldBob(alice.page).getByRole('button', { name: 'Pin', exact: true }).click();
  await boldBob(alice.page).getByText('Pinned').first().waitFor({ timeout: 5000 });
  await alice.page.getByRole('button', { name: 'Pins', exact: true }).click();
  const pinsBox = alice.page.getByRole('dialog', { name: 'Pinned messages' });
  await pinsBox.getByRole('button', { name: /^Jump to the message from alice/ }).waitFor({ timeout: 5000 });
  await pinsBox.getByRole('button', { name: 'Unpin', exact: true }).click();
  await pinsBox.getByText('Nothing is pinned yet.').waitFor({ timeout: 5000 });
  await alice.page.keyboard.press('Escape');
  await pinsBox.waitFor({ state: 'hidden' });
  console.log('composer_and_pins: ok');

  // upload_roundtrip: alice attaches a PNG and sends; bob sees it inline and it decodes as 1x1.
  await alice.page.getByLabel('Attach a file').setInputFiles({ name: 'dot.png', mimeType: 'image/png', buffer: png1x1() });
  await alice.page.getByText('Attached: dot.png').waitFor({ timeout: 5000 });
  await alice.page.getByRole('button', { name: 'Send', exact: true }).click();
  await bob.page.waitForFunction(() => document.querySelector('article img[alt="dot.png"]')?.naturalWidth === 1, null, { timeout: 5000 });
  console.log('upload_roundtrip: ok');

  // uploads_ui in a room "files": the tray refuses a file over 25 MB before any upload and cancels a running one, the drop
  // overlay shows for files only, a file card downloads by id, an image keeps its 360 x 280 box until it loads and does
  // not move the view when it loads above it, the bottom stays the bottom when the last row grows (a late image, or
  // anything: the ResizeObserver on the last row), and the viewer moves with the arrow keys and gives focus back.
  const filesRoom = await call(alice.page, 'POST', '/api/rooms', { name: 'files' });
  await roomLink(alice.page, 'files').waitFor({ timeout: 5000 });
  await openRoom(alice.page, 'files');
  const attach = alice.page.getByLabel('Attach a file');
  await attach.setInputFiles({ name: 'huge.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(25 * 1024 * 1024 + 1) });
  await alice.page.getByRole('alert').filter({ hasText: 'Files can be up to 25 MB.' }).waitFor({ timeout: 5000 });
  assert.equal(await alice.page.getByRole('button', { name: 'Retry huge.bin' }).count(), 0, 'Retry offered for a file that is too large');
  await alice.page.getByRole('button', { name: 'Remove huge.bin' }).click();
  await alice.page.route('**/api/rooms/*/uploads?*', () => {}); // held: never answered
  await attach.setInputFiles({ name: 'slow.png', mimeType: 'image/png', buffer: png1x1() });
  await alice.page.getByRole('progressbar', { name: 'Upload progress' }).waitFor({ timeout: 5000 });
  await alice.page.getByRole('button', { name: 'Cancel slow.png' }).click();
  await alice.page.getByRole('list', { name: 'Files to send' }).waitFor({ state: 'detached', timeout: 5000 });
  await alice.page.unrouteAll({ behavior: 'ignoreErrors' });
  // A room change keeps an upload going: the other room says so, and the file is attached on the way back.
  let letGo;
  await alice.page.route('**/api/rooms/*/uploads?*', async (r) => { await new Promise((ok) => (letGo = ok)); await r.continue(); });
  await attach.setInputFiles({ name: 'away.png', mimeType: 'image/png', buffer: png1x1() });
  await alice.page.getByRole('progressbar', { name: 'Upload progress' }).waitFor({ timeout: 5000 });
  await openRoom(alice.page, 'general');
  await alice.page.getByText('Uploading in files').waitFor({ timeout: 5000 });
  assert.equal(await alice.page.getByRole('list', { name: 'Files to send' }).count(), 0, 'the files upload showed in general');
  while (!letGo) await sleep(20);
  letGo();
  await alice.page.getByText('Uploading in files').waitFor({ state: 'detached', timeout: 5000 });
  await openRoom(alice.page, 'files');
  await alice.page.getByText('Attached: away.png').waitFor({ timeout: 5000 });
  await alice.page.getByRole('button', { name: 'Remove away.png' }).click();
  await alice.page.unrouteAll({ behavior: 'ignoreErrors' });
  console.log('upload_survives_room_change: ok');
  const chatPane = alice.page.getByRole('region', { name: 'Chat' });
  const drag = (kind) => alice.page.evaluateHandle((kind) => {
    const d = new DataTransfer();
    if (kind === 'file') d.items.add(new File(['some notes'], 'notes.txt', { type: 'text/plain' }));
    else d.setData('text/plain', 'just text');
    return d;
  }, kind);
  await chatPane.dispatchEvent('dragenter', { dataTransfer: await drag('text') });
  await sleep(200);
  assert.equal(await alice.page.getByText('Drop to attach').count(), 0, 'a text drag showed the drop overlay');
  const fileDrag = await drag('file');
  await chatPane.dispatchEvent('dragenter', { dataTransfer: fileDrag });
  await alice.page.getByText('Drop to attach').waitFor({ timeout: 2000 });
  await chatPane.dispatchEvent('drop', { dataTransfer: fileDrag });
  await alice.page.getByText('Drop to attach').waitFor({ state: 'detached', timeout: 2000 });
  await alice.page.getByText('Attached: notes.txt').waitFor({ timeout: 5000 });
  await alice.page.getByRole('button', { name: 'Send', exact: true }).click();
  await alice.page.getByRole('list', { name: 'Files to send' }).waitFor({ state: 'detached', timeout: 5000 });

  const uploadAs = (page, roomId, name) => page.evaluate(async ([roomId, name, bytes]) => {
    const r = await fetch(`/api/rooms/${roomId}/uploads?name=${encodeURIComponent(name)}`, { method: 'POST', body: new Uint8Array(bytes) });
    return (await r.json()).attachment.id;
  }, [roomId, name, [...png1x1()]]);
  const held = new Map(); // bob's image requests, by upload id, until the test lets each one go
  await bob.page.route('**/api/uploads/*', async (r) => { await new Promise((ok) => held.set(r.request().url().split('/').pop(), ok)); await r.continue(); });
  const release = async (id) => { while (!held.has(id)) await sleep(20); held.get(id)(); };
  const lateId = await uploadAs(alice.page, filesRoom.id, 'late.png');
  await call(alice.page, 'POST', `/api/rooms/${filesRoom.id}/messages`, { body: 'a late image', reply_to: null, attachments: [lateId] });
  const midId = await uploadAs(alice.page, filesRoom.id, 'mid.png');
  await call(alice.page, 'POST', `/api/rooms/${filesRoom.id}/messages`, { body: 'text above an image\n' + 'line\n'.repeat(10), reply_to: null, attachments: [midId] });
  for (let i = 0; i < 8; i++) await post([alice, bob][i % 2].page, filesRoom.id, `tall ${i}\n` + 'line\n'.repeat(5));
  await openRoom(bob.page, 'files');
  await bob.page.locator('article', { hasText: 'tall 7' }).waitFor({ timeout: 10000 });
  const card = bob.page.getByRole('link', { name: 'Download notes.txt' });
  assert.equal(await card.getAttribute('download'), 'notes.txt');
  assert.match(await card.getAttribute('href'), /^\/api\/uploads\/[0-9a-f]{32}$/);
  const lateBox = bob.page.getByRole('button', { name: 'Open image late.png' });
  const tl = bob.page.locator('[data-timeline]');
  // Above the view: the row ends 40 px above the top of the list. The first row whose top is in view must not move.
  await lateBox.evaluate((el) => { const row = el.closest('article'); row.parentElement.scrollTop = row.offsetTop + row.offsetHeight + 40; });
  await sleep(200);
  const box = await lateBox.evaluate((el) => [el.offsetWidth, el.offsetHeight]);
  assert.deepEqual(box, [360, 280], 'the image box was not kept before the image loaded');
  const refTop = () => tl.evaluate((el) => {
    const top = el.getBoundingClientRect().top;
    return [...el.querySelectorAll(':scope > article')].map((a) => a.getBoundingClientRect().top).find((t) => t >= top);
  });
  const before = await refTop();
  await release(lateId);
  await bob.page.waitForFunction(() => document.querySelector('img[alt="late.png"]')?.closest('button').offsetHeight === 48, null, { timeout: 5000 });
  await sleep(100);
  assert.ok(Math.abs((await refTop()) - before) < 1, `the view moved by ${(await refTop()) - before} px when the image loaded`);
  // The image is in the view but its row's text starts above it: nothing above the image moves, so neither does it.
  const midBox = bob.page.getByRole('button', { name: 'Open image mid.png' });
  await midBox.evaluate((el) => { const l = el.closest('[data-timeline]'); l.scrollTop += el.getBoundingClientRect().top - l.getBoundingClientRect().top - 40; });
  await sleep(200);
  const midTop = () => midBox.evaluate((el) => [el.getBoundingClientRect().top - el.closest('[data-timeline]').getBoundingClientRect().top, el.closest('article').getBoundingClientRect().top - el.closest('[data-timeline]').getBoundingClientRect().top]);
  const [imgAt, rowAt] = await midTop();
  assert.ok(Math.abs(imgAt - 40) < 2 && rowAt < 0, `the setup is wrong: image at ${imgAt}, row at ${rowAt}`);
  await release(midId);
  await bob.page.waitForFunction(() => document.querySelector('img[alt="mid.png"]')?.closest('button').offsetHeight === 48, null, { timeout: 5000 });
  await sleep(100);
  const [imgAfter] = await midTop();
  assert.ok(Math.abs(imgAfter - imgAt) < 1, `an image in view moved by ${imgAfter - imgAt} px when it loaded`);
  console.log('image_no_shift: ok');
  const atEnd = () => tl.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight < 8);
  await tl.evaluate((el) => (el.scrollTop = el.scrollHeight));
  await sleep(200);
  const lastId = await uploadAs(alice.page, filesRoom.id, 'last.png');
  await call(alice.page, 'POST', `/api/rooms/${filesRoom.id}/messages`, { body: 'the last image', reply_to: null, attachments: [lastId] });
  await bob.page.getByRole('button', { name: 'Open image last.png' }).waitFor({ timeout: 5000 });
  await sleep(200);
  assert.ok(await atEnd(), 'not at the bottom after a new image message');
  await release(lastId);
  await bob.page.waitForFunction(() => document.querySelector('img[alt="last.png"]')?.naturalWidth === 1, null, { timeout: 5000 });
  await sleep(200);
  assert.ok(await atEnd(), 'a late image in the last row left the bottom');
  await tl.evaluate((el) => { const d = document.createElement('div'); d.id = 'grow'; d.style.height = '300px'; el.querySelector(':scope > article:last-of-type .content').append(d); });
  await sleep(200);
  assert.ok(await atEnd(), 'a taller last row left the bottom');
  await tl.evaluate(() => document.getElementById('grow').remove());
  await bob.page.unrouteAll({ behavior: 'ignoreErrors' });
  console.log('stick_to_bottom_last_row: ok');
  await lateBox.click();
  const viewerBox = bob.page.getByRole('dialog', { name: 'late.png' });
  await viewerBox.getByText('1 of 3').waitFor({ timeout: 5000 });
  await bob.page.keyboard.press('ArrowRight');
  await bob.page.getByRole('dialog', { name: 'mid.png' }).getByText('2 of 3').waitFor({ timeout: 2000 });
  await bob.page.keyboard.press('ArrowRight');
  await bob.page.getByRole('dialog', { name: 'last.png' }).getByText('3 of 3').waitFor({ timeout: 2000 });
  const dl = bob.page.getByRole('dialog').getByRole('link', { name: 'Download' });
  assert.equal(await dl.getAttribute('download'), 'last.png');
  assert.equal(await dl.getAttribute('href'), `/api/uploads/${lastId}`);
  await bob.page.keyboard.press('Escape');
  await bob.page.getByRole('dialog').waitFor({ state: 'hidden', timeout: 2000 });
  assert.equal(await bob.page.evaluate(() => document.activeElement?.getAttribute('aria-label')), 'Open image late.png', 'focus did not go back to the image');
  console.log('uploads_ui: ok');
  for (const p of [alice.page, bob.page]) await openRoom(p, 'general');

  // search_roundtrip: a restricted room bob is not in holds another "hello". Alice (owner) finds both, bob only his.
  const room = roomId(bob.page); // general
  const secret = await call(alice.page, 'POST', '/api/rooms', { name: 'secret' });
  await call(alice.page, 'PATCH', `/api/rooms/${secret.id}`, { restricted: true });
  await call(alice.page, 'POST', `/api/rooms/${secret.id}/messages`, { body: 'hello from the secret room', reply_to: null, attachments: [] });
  const results = searchHits;
  await searchFor(alice.page, 'hello');
  await results(alice.page).nth(1).waitFor({ timeout: 5000 });
  assert.ok((await results(alice.page).allInnerTexts()).some((t) => t.includes('in secret')), 'alice (owner) should find the secret room');
  await alice.page.keyboard.press('Escape');
  await alice.page.getByRole('dialog', { name: 'Search' }).waitFor({ state: 'hidden', timeout: 2000 });
  assert.equal(await alice.page.evaluate(() => document.activeElement?.getAttribute('aria-label')), 'Search', 'Esc did not give focus back to Search');
  // Bob opens it with the keys; the filter syntax narrows it, and an unknown name says so instead of searching.
  await searchFor(bob.page, 'from:nobody hello', { keys: true });
  await bob.page.getByText('No one called nobody.').waitFor({ timeout: 5000 });
  await bob.page.getByLabel('Search messages').fill('h');
  await bob.page.getByText('Type at least 2 letters to search as you type.').waitFor({ timeout: 5000 });
  await bob.page.getByLabel('Search messages').fill('hello');
  await results(bob.page).first().waitFor({ timeout: 5000 });
  await sleep(500); // a late second hit would show by now
  assert.equal(await results(bob.page).count(), 1, 'bob should see exactly one hit');
  assert.ok(!(await results(bob.page).first().innerText()).includes('secret'), 'bob found a message from a hidden room');
  assert.equal(await results(bob.page).locator('mark').first().innerText(), 'hello');
  const links = await bob.page.getByRole('navigation', { name: 'Rooms' }).getByRole('link').allInnerTexts();
  assert.ok(links.length && !links.some((o) => o.includes('secret')), 'bob sees the hidden room');
  // The jump: enough newer messages that the hit is out of view and outside the newest page's newer half.
  for (let i = 0; i < 30; i++) await post([alice, bob][i % 2].page, room, `filler ${i}`);
  await bob.page.locator('article', { hasText: 'filler 29' }).waitFor({ timeout: 5000 });
  const shown = (loc) => loc.evaluate((el) => {
    const [r, l] = [el.getBoundingClientRect(), el.parentElement.getBoundingClientRect()];
    return r.bottom > l.top && r.top < l.bottom;
  });
  assert.equal(await shown(boldBob(bob.page)), false, 'the hit was already in view before the jump');
  // The dialog was left open while the fillers came in; Down then Enter jumps from the keyboard.
  await bob.page.getByLabel('Search messages').focus();
  await bob.page.keyboard.press('ArrowDown');
  assert.match(await bob.page.evaluate(() => document.activeElement?.textContent ?? ''), /hello/, 'Down did not reach the first hit');
  await bob.page.keyboard.press('Enter');
  await bob.page.getByRole('dialog', { name: 'Search' }).waitFor({ state: 'hidden', timeout: 2000 });
  await bob.page.waitForFunction(() => {
    const el = [...document.querySelectorAll('article')].find((a) => a.querySelector('strong')?.textContent === 'bob');
    const [r, l] = [el.getBoundingClientRect(), el.parentElement.getBoundingClientRect()];
    return r.bottom > l.top && r.top < l.bottom;
  }, null, { timeout: 5000 });
  const newer = bob.page.getByRole('button', { name: 'Load newer messages' });
  await newer.waitFor({ timeout: 5000 });
  console.log('search_roundtrip: ok');

  // rejoin_after_jump: leaving and re-entering the room shows the newest page again, with live messages.
  const lobby = await call(alice.page, 'POST', '/api/rooms', { name: 'lobby' });
  await roomLink(bob.page, 'lobby').waitFor({ timeout: 5000 });
  await openRoom(bob.page, 'lobby');
  assert.equal(roomId(bob.page), lobby.id);
  await openRoom(bob.page, 'general');
  await newer.waitFor({ state: 'detached', timeout: 5000 });
  await post(alice.page, room, 'after the jump');
  await bob.page.locator('article', { hasText: 'after the jump' }).waitFor({ timeout: 5000 });
  console.log('rejoin_after_jump: ok');

  // Scrolled up, "Jump to latest" shows; Esc (nothing focused that types) goes back to the newest message.
  const list = bob.page.locator('[data-timeline]');
  await list.evaluate((el) => (el.scrollTop = 0));
  const latest = bob.page.getByRole('button', { name: 'Jump to latest' });
  await latest.waitFor({ timeout: 5000 });
  // Esc with a message menu open closes the menu only: the list stays where it is.
  await bob.page.locator('[data-timeline] > article').first().getByRole('button', { name: 'More', exact: true }).click();
  const menu = bob.page.getByRole('menu', { name: 'Message actions' });
  await menu.waitFor({ timeout: 2000 });
  const top = await list.evaluate((el) => el.scrollTop);
  await bob.page.keyboard.press('Escape');
  await menu.waitFor({ state: 'hidden', timeout: 2000 });
  assert.equal(await list.evaluate((el) => el.scrollTop), top, 'Esc in the menu also moved the list');
  assert.ok(await latest.isVisible(), 'Esc in the menu also went to the newest message');
  await list.focus();
  await bob.page.keyboard.press('Escape');
  await latest.waitFor({ state: 'detached', timeout: 5000 });
  assert.ok(await list.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight < 8), 'Esc did not go to the newest message');
  console.log('escape_to_latest: ok');

  for (const p of [alice.page, bob.page]) assert.deepEqual(await cspViolations(p), [], 'CSP violations');
  console.log('no CSP violations: ok');
  assert.deepEqual([...alice.errors, ...bob.errors], [], 'page errors');
  console.log('e2e two-browser: PASS');
} finally {
  await stop();
}
