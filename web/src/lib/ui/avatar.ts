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

// The PATCH /api/me body for a new name and slot: only what changed. The slot is compared with the stored accent, not
// with avatarSlot(): the server default is no slot, so a colour equal to the id-hash slot must still be saved.
export function profilePatch(u: { display_name: string; accent: string }, name: string, slot: number): { display_name?: string; accent?: string } {
  const patch: { display_name?: string; accent?: string } = {};
  if (name.trim() !== u.display_name) patch.display_name = name;
  if (SLOT_HEX[slot - 1] !== u.accent.trim().toLowerCase()) patch.accent = SLOT_HEX[slot - 1];
  return patch;
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
