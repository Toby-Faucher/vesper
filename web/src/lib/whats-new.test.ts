import { describe, expect, it } from 'vitest';
import { RELEASE_NOTES, whatsNew, type Note } from './release-notes';
import { SEEN_KEY, newer, takeSeen } from './whats-new';

const NOTES: Note[] = [
  { version: '1.8.0', items: ['a'] },
  { version: '1.8.2', items: ['c'] },
  { version: '1.8.1', items: ['b'] },
];
const versions = (n: Note[] | null) => n?.map((x) => x.version) ?? null;

describe('whatsNew', () => {
  it('shows_nothing_on_a_first_visit', () => expect(whatsNew(null, '1.8.2', NOTES)).toBeNull());
  it('shows_the_one_version_after_an_upgrade_by_one', () => expect(versions(whatsNew('1.8.1', '1.8.2', NOTES))).toEqual(['1.8.2']));
  it('shows_every_skipped_version_newest_first', () => expect(versions(whatsNew('1.7.2', '1.8.2', NOTES))).toEqual(['1.8.2', '1.8.1', '1.8.0']));
  it('leaves_out_versions_after_the_current_one', () => expect(versions(whatsNew('1.7.2', '1.8.1', NOTES))).toEqual(['1.8.1', '1.8.0']));
  it('shows_nothing_on_the_same_version_or_a_rollback', () => {
    expect(whatsNew('1.8.2', '1.8.2', NOTES)).toBeNull();
    expect(whatsNew('1.8.2', '1.8.1', NOTES)).toBeNull();
  });
  it('shows_nothing_without_notes_for_those_versions', () => {
    expect(whatsNew('1.8.2', '1.9.0', NOTES)).toBeNull();
    expect(whatsNew('1.7.0', '1.8.2', [])).toBeNull();
  });
  it('shows_nothing_for_a_bad_stored_value', () => {
    for (const bad of ['', 'garbage', '1.8', '1.8.0-rc1', 'v1.8.0', '{}']) expect(whatsNew(bad, '1.8.2', NOTES)).toBeNull();
    expect(whatsNew('1.8.0', 'dev', NOTES)).toBeNull();
  });
  it('compares_by_number_not_as_text', () => {
    expect(newer('1.10.0', '1.9.0')).toBe(true);
    expect(newer('1.9.0', '1.10.0')).toBe(false);
    expect(newer('2.0.0', '1.99.99')).toBe(true);
    expect(versions(whatsNew('1.9.0', '1.10.0', [{ version: '1.10.0', items: ['x'] }, { version: '1.9.0', items: ['y'] }]))).toEqual(['1.10.0']);
  });
});

describe('takeSeen', () => {
  const store = (v: string | null) => {
    const m = new Map<string, string>(v === null ? [] : [[SEEN_KEY, v]]);
    return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, x: string) => void m.set(k, x), m };
  };
  it('returns_the_old_version_after_an_upgrade_and_stores_the_new_one', () => {
    const s = store('1.8.1');
    expect(takeSeen(s, '1.8.2')).toBe('1.8.1');
    expect(s.m.get(SEEN_KEY)).toBe('1.8.2');
    expect(takeSeen(s, '1.8.2')).toBeNull(); // shown once
  });
  it('stores_the_current_version_but_returns_null_otherwise', () => {
    for (const before of [null, '1.8.2', '1.9.0', 'garbage']) {
      const s = store(before);
      expect(takeSeen(s, '1.8.2')).toBeNull(); // first visit, same, rollback, bad value
      expect(s.m.get(SEEN_KEY)).toBe('1.8.2');
    }
  });
  it('returns_null_when_storage_is_blocked', () => {
    expect(takeSeen(null, '1.8.2')).toBeNull();
    const thrown = { getItem: () => { throw new Error('blocked'); }, setItem: () => {} };
    expect(takeSeen(thrown, '1.8.2')).toBeNull();
  });
});

describe('RELEASE_NOTES', () => {
  it('has_one_entry_per_version_each_with_short_lines', () => {
    const vs = RELEASE_NOTES.map((n) => n.version);
    expect(new Set(vs).size).toBe(vs.length);
    for (const n of RELEASE_NOTES) {
      expect(n.version).toMatch(/^\d+\.\d+\.\d+$/);
      expect(n.items.length).toBeGreaterThan(0);
      for (const i of n.items) expect(i.length).toBeLessThanOrEqual(80);
    }
  });
});
