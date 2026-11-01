import { expect, it } from 'vitest';
import tokens from '../../styles/tokens.css?raw';
import { SLOT_HEX, avatarSlot, initials, profilePatch } from './avatar';

// A fixed pseudo-random sequence of 32-hex ids, so the spread test cannot flake.
function ids(n: number): string[] {
  let s = 12345;
  return Array.from({ length: n }, () =>
    Array.from({ length: 4 }, () => ((s = (Math.imul(s, 1103515245) + 12345) >>> 0), s.toString(16).padStart(8, '0'))).join(''),
  );
}

// SLOT_HEX must be exactly --avatar-1..8 of the vesper style, in order: parsed from tokens.css, not searched for.
it('slot_hex_matches_the_vesper_tokens', () => {
  const block = tokens.slice(tokens.indexOf("[data-style='vesper']"));
  const parsed = [...block.slice(0, block.indexOf('}')).matchAll(/--avatar-(\d+)\s*:\s*([^;]+);/g)];
  expect(parsed.map((m) => Number(m[1]))).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  expect(parsed.map((m) => m[2].trim().toLowerCase())).toEqual([...SLOT_HEX]);
});

it('swatch_hex_gives_its_slot', () => {
  SLOT_HEX.forEach((h, i) => expect(avatarSlot({ user_id: 'a'.repeat(32), accent: h })).toBe(i + 1));
  expect(avatarSlot({ user_id: 'a'.repeat(32), accent: SLOT_HEX[4].toUpperCase() })).toBe(5);
});

it('other_values_hash_the_id', () => {
  const [id] = ids(1);
  const a = avatarSlot({ user_id: id, accent: '#7c8cff' });
  expect(avatarSlot({ user_id: id, accent: 'not a colour' })).toBe(a);
  expect(avatarSlot({ user_id: id, accent: '' })).toBe(a);
  const seen = new Set(ids(100).map((user_id) => avatarSlot({ user_id, accent: '#7c8cff' })));
  expect([...seen].sort()).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
});

it('initials_by_grapheme', () => {
  expect(initials('Émile Zola')).toBe('ÉZ');
  expect(initials('Émile Zola')).toBe('ÉZ');
  expect(initials('toby')).toBe('T');
  expect(initials('  ana   maria lopez ')).toBe('AM');
  expect(initials('👩‍👩‍👧 family')).toBe('👩‍👩‍👧F');
  expect(initials('🇫🇷')).toBe('🇫🇷');
  expect(initials('')).toBe('');
});

it('profile_patch_saves_the_hashed_slot_over_the_server_default', () => {
  const [id] = ids(1);
  const u = { user_id: id, display_name: 'bob', accent: '#7c8cff' };
  const slot = avatarSlot(u); // the colour shown before any choice
  expect(profilePatch(u, 'bob', slot)).toEqual({ accent: SLOT_HEX[slot - 1] });
  expect(profilePatch({ ...u, accent: SLOT_HEX[2].toUpperCase() }, ' bob ', 3)).toEqual({});
  expect(profilePatch({ ...u, accent: SLOT_HEX[2] }, 'Bobby', 3)).toEqual({ display_name: 'Bobby' });
});
