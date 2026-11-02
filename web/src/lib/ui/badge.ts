import type { Unread } from '../gen/Unread';

type Badging = { setAppBadge?: (n?: number) => Promise<void>; clearAppBadge?: () => Promise<void> };
let shown = -1; // unknown: the worker may have set a badge, so the first count is always sent

/** The installed app's badge shows the unread mentions. Only where the browser has the Badging API (Chromium on Windows
 * and macOS, Safari 17 for installed apps); Firefox has none, so nothing happens there. After the first count, only a
 * changed count is sent. */
export function badge(unread: Record<number, Unread>, nav: Badging = navigator as Badging): void {
  const n = Object.values(unread).reduce((sum, u) => sum + u.mentions, 0);
  if (n === shown) return;
  shown = n;
  // A refusal (not installed, no permission) changes nothing for the page.
  (n ? nav.setAppBadge?.(n) : nav.clearAppBadge?.())?.catch(() => {});
}

/** Sign-out: clear the badge now and send the next account's first count whatever it is. */
export function resetBadge(nav: Badging = navigator as Badging): void {
  shown = -1;
  badge({}, nav);
  shown = -1;
}
