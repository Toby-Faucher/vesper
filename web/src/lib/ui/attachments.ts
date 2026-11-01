// Helpers for files in messages and in the upload tray. Names and types come from users: shown as text only.

/** The largest box an image takes in the timeline. Space for it is kept before the image loads. */
export const BOX = { w: 360, h: 280 };
const MIN = 48; // a tiny image still gets a box you can click

/** "12 KB", "3.4 MB": binary units, as the server counts its 25 MB limit, rounded up. */
export const fileSize = (n: number) =>
  n < 1024 * 1024 ? `${Math.max(1, Math.ceil(n / 1024))} KB` : `${(Math.ceil((n / 1024 / 1024) * 10) / 10).toFixed(1)} MB`; // up: just over 25 MB never reads 25.0

/** The box for an image of natural size w x h: scaled down to fit BOX, never up, at least MIN on each side. */
export function fit(w: number, h: number): { w: number; h: number } {
  if (!(w > 0 && h > 0)) return { ...BOX };
  const s = Math.min(1, BOX.w / w, BOX.h / h);
  return { w: Math.max(MIN, Math.round(w * s)), h: Math.max(MIN, Math.round(h * s)) };
}

const TYPES: Record<string, string> = { 'image/png': 'PNG image', 'image/jpeg': 'JPEG image', 'image/gif': 'GIF image', 'image/webp': 'WebP image' };
/** The label for the type the server found in the first bytes. It only tells images apart; anything else is a file. */
export const typeLabel = (mime: string) => TYPES[mime] ?? 'File';

/**
 * Runs `change` (which resizes something inside a timeline row) without moving what the reader sees: when the list is
 * at the bottom it stays there; when the row starts above the visible part, the list scrolls by the height change. For
 * that case the browser's own scroll anchoring is off, so the two never both correct. (Only then: a scroll position
 * that does not change keeps Chromium's old anchor, which it applied again on the next layout.)
 */
export async function keepView(el: Element, change: () => Promise<unknown>) {
  const list = el.closest<HTMLElement>('[data-timeline]');
  const row = el.closest('article');
  if (!list || !row) return void (await change());
  const bottom = list.scrollHeight - list.scrollTop - list.clientHeight < 8;
  const above = !bottom && row.getBoundingClientRect().top < list.getBoundingClientRect().top;
  const before = row.getBoundingClientRect().height;
  if (above) list.style.overflowAnchor = 'none';
  await change();
  if (bottom) {
    // A real scroll, even when the shrink already clamped the list to the bottom: Chromium keeps its anchor through a
    // clamp and applied the shrink once more on the next layout, which left the bottom.
    list.scrollTop = list.scrollHeight - list.clientHeight - 1;
    list.scrollTop = list.scrollHeight;
  }
  if (!above) return;
  list.scrollTop += row.getBoundingClientRect().height - before;
  requestAnimationFrame(() => (list.style.overflowAnchor = ''));
}
