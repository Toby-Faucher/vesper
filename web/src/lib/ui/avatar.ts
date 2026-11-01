// Per-user colour: one of 8 avatar slots. UserInfo.accent holds the vesper swatch hex of the chosen slot; any other value
// (the server default #7c8cff, a custom hex, garbage) falls back to a hash of the user id, so it is stable per person.
// The slot is drawn with var(--avatar-N), so another style can redraw the slots without touching this file.
// These hexes are the only colour literals outside styles/tokens.css (no-raw-colours.test.ts skips this file).
export const SLOT_HEX: readonly string[] = ['#ed8d7d', '#e5955d', '#99b860', '#6fc082', '#31c3b5', '#41bae4', '#6db0f4', '#e68aaf'];

// FNV-1a, 32 bit.
function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return h >>> 0;
}

export function avatarSlot(u: { user_id: string; accent: string }): number {
  const i = SLOT_HEX.indexOf(u.accent.trim().toLowerCase());
  return i >= 0 ? i + 1 : (hash(u.user_id) % SLOT_HEX.length) + 1;
}

// The PATCH /api/me body for a new name, slot and bio: only what changed. The slot is compared with the stored accent, not
// with avatarSlot(): the server default is no slot, so a colour equal to the id-hash slot must still be saved.
export function profilePatch(
  u: { display_name: string; accent: string; bio?: string },
  name: string,
  slot: number,
  bio?: string,
): { display_name?: string; accent?: string; bio?: string } {
  const patch: { display_name?: string; accent?: string; bio?: string } = {};
  if (name.trim() !== u.display_name) patch.display_name = name;
  if (SLOT_HEX[slot - 1] !== u.accent.trim().toLowerCase()) patch.accent = SLOT_HEX[slot - 1];
  if (bio !== undefined && bio.trim().split(/\s+/).join(' ') !== (u.bio ?? '')) patch.bio = bio;
  return patch;
}

export const MAX_BIO = 160;

// The picture of a user, or null for the letter avatar. The version in the URL changes with the picture, so it caches.
export const avatarUrl = (u: { user_id: string; avatar?: string | null }): string | null =>
  u.avatar ? `/api/users/${u.user_id}/avatar?v=${encodeURIComponent(u.avatar)}` : null;

// Profile pictures: any of these is cropped to the centred square and redrawn at PICTURE_PX, so the server gets a small
// still WebP (or PNG where the browser cannot write WebP). The server takes at most 512 KiB.
export const PICTURE_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif'];
export const PICTURE_PX = 256;
export const MAX_PICTURE = 512 * 1024;

/** The centred square of a w x h picture, as [x, y, side]. */
export function squareCrop(w: number, h: number): [number, number, number] {
  const side = Math.min(w, h);
  return [Math.floor((w - side) / 2), Math.floor((h - side) / 2), side];
}

/** The file as a PICTURE_PX square. Rejects with 'type' for a file that is not a picture type, 'unreadable' when the
 *  browser cannot decode or encode it. */
export async function squarePicture(file: Blob): Promise<Blob> {
  if (!PICTURE_TYPES.includes(file.type)) throw new Error('type');
  const bmp = await createImageBitmap(file).catch(() => {
    throw new Error('unreadable');
  });
  const [x, y, side] = squareCrop(bmp.width, bmp.height);
  const c = document.createElement('canvas');
  c.width = c.height = PICTURE_PX;
  const g = c.getContext('2d');
  if (!g || !side) throw new Error('unreadable');
  g.imageSmoothingQuality = 'high';
  g.drawImage(bmp, x, y, side, side, 0, 0, PICTURE_PX, PICTURE_PX);
  bmp.close();
  const out = await new Promise<Blob | null>((done) => c.toBlob(done, 'image/webp', 0.9));
  if (!out) throw new Error('unreadable');
  return out;
}

const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

// The first grapheme of the first two words, upper-cased. NFC first, so a combining accent matches the precomposed letter.
export function initials(name: string): string {
  return name
    .normalize('NFC')
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => graphemes.segment(w)[Symbol.iterator]().next().value?.segment ?? '')
    .join('')
    .toUpperCase();
}
