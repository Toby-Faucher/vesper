// Helpers for files in messages and in the upload tray. Names and types come from users: shown as text only.
import type { Attachment } from '../gen/Attachment';
import type { Message } from '../gen/Message';
import type { IconPath } from './icons';

// The video card's open button. Here, not in icons.ts, so it stays in the lazy chat chunk (icons.ts is in the first load).
// Lucide `maximize` (lucide-static 0.460.0, ISC licence, see icons.ts).
export const expandIcon = 'M8 3H5a2 2 0 0 0-2 2v3M21 8V5a2 2 0 0 0-2-2h-3M3 16v3a2 2 0 0 0 2 2h3M16 21h3a2 2 0 0 0 2-2v-3' as IconPath;

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
/**
 * The card's type label. Images by the type the server found in the first bytes; any other file (the server calls them
 * all application/octet-stream) by the extension of its name, "TXT file", which only repeats what the name says. A
 * name with no short extension gets no label (empty).
 */
export function typeLabel(mime: string, name = ''): string {
  if (TYPES[mime]) return TYPES[mime];
  const ext = /\.([a-z0-9]{1,5})$/i.exec(name)?.[1];
  return ext ? `${ext.toUpperCase()} file` : '';
}

/**
 * What keepView does for an image whose box is about to change, from where it is now: `gap` is the list's distance
 * from the bottom, `top` the image's top and `listTop` the list's top (both from getBoundingClientRect).
 * - 'bottom': the list is at the bottom and stays there.
 * - 'above': the image starts above the visible part (fully above, or across the top edge): what is below it would
 *   move, so the list scrolls by the change. The image's own top counts, not its row's: a row whose text starts
 *   above the view but whose image is in it needs nothing.
 * - 'none': the image is in the view or below it; nothing above it moves.
 */
export function viewCase(gap: number, top: number, listTop: number): 'bottom' | 'above' | 'none' {
  if (gap < 8) return 'bottom';
  return top < listTop ? 'above' : 'none';
}

/**
 * Runs `change` (which resizes the image `el` inside a timeline row) without moving what the reader sees (viewCase).
 * For 'above' the browser's own scroll anchoring is off, so the two never both correct. (Only then: a scroll position
 * that does not change keeps Chromium's old anchor, which it applied again on the next layout.)
 */
export async function keepView(el: Element, change: () => Promise<unknown>) {
  const list = el.closest<HTMLElement>('[data-timeline]');
  const row = el.closest('article');
  if (!list || !row) return void (await change());
  const how = viewCase(list.scrollHeight - list.scrollTop - list.clientHeight, el.getBoundingClientRect().top, list.getBoundingClientRect().top);
  const before = row.getBoundingClientRect().height;
  if (how === 'above') list.style.overflowAnchor = 'none';
  await change();
  if (how === 'bottom') {
    // A real scroll, even when the shrink already clamped the list to the bottom: Chromium keeps its anchor through a
    // clamp and applied the shrink once more on the next layout, which left the bottom. The 1 px step up stays within
    // the bottom (Timeline's follows() counts 8 px), so it never unsticks the list.
    list.scrollTop = list.scrollHeight - list.clientHeight - 1;
    list.scrollTop = list.scrollHeight;
  }
  if (how !== 'above') return;
  list.scrollTop += row.getBoundingClientRect().height - before;
  requestAnimationFrame(() => (list.style.overflowAnchor = ''));
}

const EXT: Record<string, string> = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' };
const two = (n: number) => String(n).padStart(2, '0');

/** The name browsers give every pasted clipboard image. A pasted file with another name is a real file and keeps it. */
export const isGenericPaste = (name: string) => /^image\.[a-z0-9]{1,5}$/i.test(name);

/**
 * A readable name for a pasted clipboard image ("Pasted image 2026-11-03 14.05.png", local time), with " (2)", " (3)" when
 * `taken` already holds it. The extension follows the MIME type. Plain characters only, far under the server's 200.
 */
export function pastedName(mime: string, now: Date, taken: ReadonlySet<string>): string {
  const base = `Pasted image ${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())} ${two(now.getHours())}.${two(now.getMinutes())}`;
  const ext = EXT[mime] ?? 'png';
  let name = `${base}.${ext}`;
  for (let n = 2; taken.has(name); n++) name = `${base} (${n}).${ext}`;
  return name;
}

/** What the viewer walks: the images and videos of `messages` in order, deleted messages left out. Audio stays inline. */
export const viewable = (messages: Message[]): Attachment[] =>
  messages.filter((m) => !m.deleted).flatMap((m) => m.attachments.filter((a) => a.kind === 'image' || a.kind === 'video'));
