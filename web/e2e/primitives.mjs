// The keyboard paths of the primitives in lib/ui/c, on the dev-only /_gallery?primitives page: a focus ring on every Tab
// stop, Dialog focus in and back, Esc, a late close event, nested dialogs and the scroll lock, Menu roving focus and
// typeahead, Popover Esc, aria-haspopup/controls/expanded, and the Popover fallback without showPopover({ source }).
// open(url) returns a new page. Throws on the first failure. ui.mjs runs it (CI); the shots `gallery` scene only shoots.
import { sleep } from './lib.mjs';

export async function checkPrimitives(open, url) {
  const page = await open(url);
  // Chromium gives focus back itself when a modal <dialog> or a popover with focus inside closes; Safari and Firefox lean
  // on Dialog's and Popover's own code. Take that away: focus leaves before a popover hides, and after a dialog closes.
  await page.addInitScript(() => {
    const blur = () => document.activeElement instanceof HTMLElement && document.activeElement.blur();
    addEventListener('beforetoggle', (e) => e.newState === 'closed' && e.target.contains(document.activeElement) && blur(), true);
    const close = HTMLDialogElement.prototype.close;
    HTMLDialogElement.prototype.close = function (...a) { close.apply(this, a); blur(); };
    // Closing a dialog or a popover leaves a fading empty box (motion.ts ghost): count them, and any text in them.
    window.__ghosts = { n: 0, text: 0 };
    new MutationObserver((ms) => ms.forEach((m) => m.addedNodes.forEach((n) => {
      if (n.dataset?.ghost === undefined) return;
      window.__ghosts.n++;
      window.__ghosts.text += n.textContent.length;
    }))).observe(document, { childList: true, subtree: true });
  });
  const ghosts = () => page.evaluate(() => ({ ...window.__ghosts }));
  await page.reload();
  await page.getByRole('heading', { name: 'Primitives' }).waitFor();
  const must = (ok, what) => { if (!ok) throw new Error(`gallery: ${what}`); };
  const focused = () => page.evaluate(() => { const a = document.activeElement; return a?.getAttribute('aria-label') || a?.textContent?.trim() || a?.tagName; });
  // Polls a little: after a light dismiss, Popover gives focus back in the toggle event, a task after the layer hides.
  const expectFocus = async (name, what) => {
    let f;
    for (let i = 0; i < 20 && (f = await focused()) !== name; i++) await sleep(50);
    must(f === name, `${what}: focus on ${JSON.stringify(f?.slice(0, 80))}, want ${JSON.stringify(name)}`);
  };

  // Every Tab stop shows an outline. Stops once focus comes back to the first element.
  const tabStops = [];
  for (let i = 0; i < 300; i++) {
    await page.keyboard.press('Tab');
    const s = await page.evaluate(() => {
      const a = document.activeElement;
      if (!a || a === document.body) return null;
      const cs = getComputedStyle(a);
      return { id: a.outerHTML.slice(0, 80), name: a.getAttribute('aria-label') || a.textContent?.trim().slice(0, 30) || a.tagName, outline: cs.outlineStyle !== 'none' && cs.outlineWidth !== '0px' };
    });
    if (!s) continue;
    if (tabStops.length && s.id === tabStops[0].id) break;
    tabStops.push(s);
  }
  const bare = tabStops.filter((s) => !s.outline).map((s) => s.name);
  must(tabStops.length > 30, `only ${tabStops.length} Tab stops`);
  must(!bare.length, `no focus outline on ${bare.join(', ')}`);
  console.log(`gallery: ${tabStops.length} Tab stops, each with a focus ring`);

  // Dialog: focus moves in, Esc closes, focus returns; a backdrop click closes too.
  const opener = page.getByRole('button', { name: 'Create invite' });
  const dialog = page.getByRole('dialog', { name: 'Create an invite' });
  await opener.click();
  await dialog.waitFor();
  must(await dialog.evaluate((d) => d.contains(document.activeElement)), 'dialog did not take focus');
  const g0 = (await ghosts()).n;
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
  await expectFocus('Create invite', 'after Esc on the dialog');
  must((await ghosts()).n === g0 + 1, 'closing the dialog left no fading box');
  await opener.click();
  await dialog.waitFor();
  await page.mouse.click(8, 8);
  await dialog.waitFor({ state: 'hidden' });
  await expectFocus('Create invite', 'after a backdrop click');
  // Esc, then open again at once: the close event of the first close arrives late and must not close the new one, and
  // onclose ran once per close (Esc and the backdrop click: 2).
  const closes = () => page.getByRole('status').filter({ hasText: 'Last action' }).getAttribute('data-closes').then(Number);
  const before = await closes();
  await opener.click();
  await dialog.waitFor();
  await page.keyboard.press('Escape');
  await opener.click();
  await dialog.evaluate((d) => d.dispatchEvent(new Event('close'))); // a late close event from the Esc
  await sleep(100);
  must(await dialog.isVisible(), 'a late close event closed the reopened dialog');
  must((await closes()) === before + 1, `onclose ran ${(await closes()) - before} times for one Esc`);
  must(before === 2, `onclose ran ${before} times for Esc and a backdrop click`);
  // Removed while open (a re-key, a sign out): focus still goes back to the opener.
  await page.evaluate(() => window.__unmountDialog());
  await dialog.waitFor({ state: 'detached' });
  await expectFocus('Create invite', 'after the open dialog was removed');
  await page.reload(); // the dialog is gone from this page now
  await page.getByRole('button', { name: 'Create invite' }).waitFor();

  // Menu: roving focus, typeahead, Esc, Enter, Tab, and the trigger toggles it.
  const trigger = page.getByRole('button', { name: 'Message actions' });
  const menu = page.getByRole('menu', { name: 'Message actions' });
  must((await trigger.getAttribute('aria-haspopup')) === 'menu', 'menu trigger has no aria-haspopup');
  await trigger.click();
  await menu.waitFor();
  must((await trigger.getAttribute('aria-expanded')) === 'true', 'aria-expanded not true while open');
  await expectFocus('Reply', 'menu open');
  for (const [key, want] of [['ArrowDown', 'Edit'], ['End', 'Delete'], ['ArrowDown', 'Reply'], ['ArrowUp', 'Delete'], ['ArrowUp', 'Copy link'], ['Home', 'Reply'], ['p', 'Pin'], ['c', 'Copy link']]) {
    await page.keyboard.press(key);
    await expectFocus(want, `menu after ${key}`);
  }
  await page.keyboard.press('Escape');
  await menu.waitFor({ state: 'hidden' });
  await expectFocus('Message actions', 'after Esc on the menu');
  // The toggle event that reports the close is async: wait for the attribute.
  await page.waitForFunction(() => document.querySelector('[aria-label="Message actions"]')?.getAttribute('aria-expanded') === 'false', null, { timeout: 2000 })
    .catch(() => must(false, 'aria-expanded not false after close'));
  await page.keyboard.press('Enter');
  await menu.waitFor();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await menu.waitFor({ state: 'hidden' });
  await page.getByText('Last action: Edit').waitFor();
  await expectFocus('Message actions', 'after choosing an item');
  await page.keyboard.press('Space');
  await menu.waitFor();
  await page.keyboard.press('Tab');
  await menu.waitFor({ state: 'hidden' });
  await expectFocus('Message actions', 'after Tab in the menu');
  await trigger.click();
  await menu.waitFor();
  await trigger.click();
  await menu.waitFor({ state: 'hidden' });

  // Popover: Esc closes and focus returns.
  const card = page.getByRole('button', { name: 'Show person card' });
  await card.click();
  await page.getByRole('dialog', { name: 'bob' }).waitFor();
  const g1 = (await ghosts()).n;
  await page.keyboard.press('Escape');
  await page.getByRole('dialog', { name: 'bob' }).waitFor({ state: 'hidden' });
  await expectFocus('Show person card', 'after Esc on the popover');
  const g = await ghosts();
  must(g.n === g1 + 1, 'closing the popover left no fading box');
  must(g.text === 0, 'a fading box holds text: getByText would find it twice');

  // aria: each trigger names its popup kind and points aria-controls at the layer it opens.
  for (const [name, popup, role] of [['Message actions', 'menu', 'menu'], ['Show person card', 'dialog', 'dialog'], ['Create invite', 'dialog', 'dialog']]) {
    const t = page.getByRole('button', { name, exact: true });
    must((await t.getAttribute('aria-haspopup')) === popup, `${name}: aria-haspopup is not ${popup}`);
    const id = await t.getAttribute('aria-controls');
    must(id && (await page.evaluate((id) => document.getElementById(id)?.getAttribute('role') ?? document.getElementById(id)?.tagName.toLowerCase(), id)) === role,
      `${name}: aria-controls ${JSON.stringify(id)} does not point at a ${role}`);
  }

  // Nested dialogs and the scroll lock: the page does not scroll behind a modal, Esc closes only the top dialog and focus
  // goes back to that dialog's opener; the lock goes with the last dialog.
  const overflow = () => page.evaluate(() => getComputedStyle(document.documentElement).overflow);
  const revoke = page.getByRole('dialog', { name: 'Revoke all invite links?' });
  must((await overflow()) !== 'hidden', 'page locked before any dialog');
  await opener.click();
  await dialog.waitFor();
  must((await overflow()) === 'hidden', 'no scroll lock with a dialog open');
  const y = await page.evaluate(() => scrollY);
  await page.mouse.wheel(0, 600);
  await sleep(200);
  must((await page.evaluate(() => scrollY)) === y, 'the page scrolled behind the dialog');
  await page.getByRole('button', { name: 'Revoke all links' }).click();
  await revoke.waitFor();
  must(await revoke.evaluate((d) => d.contains(document.activeElement)), 'the inner dialog did not take focus');
  await page.keyboard.press('Escape');
  await revoke.waitFor({ state: 'hidden' });
  must(await dialog.isVisible(), 'Esc on the inner dialog closed the outer one');
  await expectFocus('Revoke all links', 'after Esc on the inner dialog');
  must((await overflow()) === 'hidden', 'scroll lock released while the outer dialog is open');
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'hidden' });
  await expectFocus('Create invite', 'after Esc on the outer dialog');
  must((await overflow()) !== 'hidden', 'scroll lock kept after the last dialog closed');

  // A browser without showPopover({ source }): light dismiss on the anchor's pointerdown must not reopen on its click.
  const old = await open(`${url}`);
  await old.addInitScript(() => {
    const show = HTMLElement.prototype.showPopover;
    HTMLElement.prototype.showPopover = function () { return show.call(this); };
  });
  await old.reload();
  const oldTrigger = old.getByRole('button', { name: 'Message actions' });
  const oldMenu = old.getByRole('menu', { name: 'Message actions' });
  await oldTrigger.click();
  await oldMenu.waitFor();
  await oldTrigger.click();
  await oldMenu.waitFor({ state: 'hidden' });
  await sleep(300);
  must(!(await oldMenu.isVisible()), 'without source: a second click on the trigger reopened the menu');
  await oldTrigger.click();
  await oldMenu.waitFor().catch(() => must(false, 'without source: the third click did not open the menu'));
  await old.keyboard.press('Escape');
  await oldMenu.waitFor({ state: 'hidden' });
  await old.close();
  await page.close();
}
