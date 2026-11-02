import { describe, expect, it } from 'vitest';
import { UI_MESSAGES, relTime } from './messages';

const NOW = Date.UTC(2026, 10, 1, 12, 0, 0);
const MIN = 60_000;

describe('relTime', () => {
  it('counts_up_in_steps', () => {
    expect(relTime(NOW - 30_000, NOW)).toBe('just now');
    expect(relTime(NOW + 5000, NOW)).toBe('just now'); // a clock a little ahead
    expect(relTime(NOW - MIN, NOW)).toBe('1 min ago');
    expect(relTime(NOW - 5 * MIN, NOW)).toBe('5 min ago');
    expect(relTime(NOW - 59 * MIN, NOW)).toBe('59 min ago');
    expect(relTime(NOW - 60 * MIN, NOW)).toBe('1 h ago');
    expect(relTime(NOW - 3 * 60 * MIN, NOW)).toBe('3 h ago');
    expect(relTime(NOW - 24 * 60 * MIN, NOW)).toBe('1 day ago');
    expect(relTime(NOW - 2 * 24 * 60 * MIN, NOW)).toBe('2 days ago');
    expect(relTime(NOW - 6 * 24 * 60 * MIN, NOW)).toBe('6 days ago');
  });
  it('gives_a_date_after_a_week', () => {
    const t = NOW - 7 * 24 * 60 * MIN;
    expect(relTime(t, NOW)).toBe(new Date(t).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' }));
    expect(relTime(t, NOW)).not.toMatch(/ago/);
  });
});

describe('resetNotice', () => {
  it('names the maker with the unique @username, so a copied display name does not pass', () => {
    expect(UI_MESSAGES.resetNotice('Toby', 'mallory', '2 Nov')).toBe('Your password was reset by Toby (@mallory) on 2 Nov.');
    expect(UI_MESSAGES.resetNotice(null, null, '2 Nov')).toBe('Your password was reset from the server command line on 2 Nov.');
  });
});
