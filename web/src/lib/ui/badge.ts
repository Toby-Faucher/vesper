import type { Unread } from '../gen/Unread';

type Badging = { setAppBadge?: (n?: number) => Promise<void>; clearAppBadge?: () => Promise<void> };
let shown = 0;

/** The installed app's badge shows the unread mentions. Only where the browser has the Badging API (Chromium on Windows
 * and macOS, Safari 17 for installed apps); Firefox has none, so nothing happens there. Only a changed count is sent. */
export function badge(unread: Record<number, Unread>, nav: Badging = navigator as Badging): void {
  const n = Object.values(unread).reduce((sum, u) => sum + u.mentions, 0);
  if (n === shown) return;
  shown = n;
  // A refusal (not installed, no permission) changes nothing for the page.
  (n ? nav.setAppBadge?.(n) : nav.clearAppBadge?.())?.catch(() => {});
}
