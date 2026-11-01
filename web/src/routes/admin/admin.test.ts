import { describe, expect, it } from 'vitest';
import type { InviteInfo } from '../../lib/gen/InviteInfo';
import { byRole, bytes, inviteStatus, moveRoom, untilText } from './admin';

const NOW = 1_800_000_000_000;
const inv = (o: Partial<InviteInfo> = {}): InviteInfo =>
  ({ id: 1, role: 'member', uses: 0, max_uses: 1, expires_at: NOW + 1000, created_by: null, revoked: false, ...o });

describe('inviteStatus', () => {
  it('active until used up, expired or revoked', () => {
    expect(inviteStatus(inv(), NOW)).toBe('active');
    expect(inviteStatus(inv({ uses: 4, max_uses: 5 }), NOW)).toBe('active');
    expect(inviteStatus(inv({ uses: 1 }), NOW)).toBe('used');
    expect(inviteStatus(inv({ expires_at: NOW }), NOW)).toBe('expired');
    expect(inviteStatus(inv({ revoked: true }), NOW)).toBe('revoked');
  });
  it('revoked, then used up, win over expired', () => {
    expect(inviteStatus(inv({ revoked: true, uses: 1, expires_at: NOW - 1 }), NOW)).toBe('revoked');
    expect(inviteStatus(inv({ uses: 1, expires_at: NOW - 1 }), NOW)).toBe('used');
  });
});

describe('untilText', () => {
  it('future in min, h and days; past through relTime', () => {
    expect(untilText(NOW + 30_000, NOW)).toBe('in less than a minute');
    expect(untilText(NOW + 59 * 60_000, NOW)).toBe('in 59 min');
    expect(untilText(NOW + 60 * 60_000, NOW)).toBe('in 1 h');
    expect(untilText(NOW + 24 * 3_600_000, NOW)).toBe('in 1 day');
    expect(untilText(NOW + 168 * 3_600_000, NOW)).toBe('in 7 days');
    expect(untilText(NOW - 2 * 3_600_000, NOW)).toBe('2 h ago');
    expect(untilText(NOW, NOW)).toBe('just now');
  });
});

describe('byRole', () => {
  it('owner, moderators, members, then by name', () => {
    const u = (display_name: string, role: InviteInfo['role']) => ({ display_name, role });
    const l = [u('zed', 'member'), u('amy', 'member'), u('bob', 'moderator'), u('tob', 'owner')].sort(byRole);
    expect(l.map((x) => x.display_name)).toEqual(['tob', 'bob', 'amy', 'zed']);
  });
});

describe('moveRoom', () => {
  it('swaps with the neighbour, keeps the ends', () => {
    expect(moveRoom([1, 2, 3], 3, -1)).toEqual([1, 3, 2]);
    expect(moveRoom([1, 2, 3], 1, 1)).toEqual([2, 1, 3]);
    expect(moveRoom([1, 2, 3], 1, -1)).toEqual([1, 2, 3]);
    expect(moveRoom([1, 2, 3], 3, 1)).toEqual([1, 2, 3]);
    expect(moveRoom([1, 2, 3], 9, 1)).toEqual([1, 2, 3]);
  });
});

describe('bytes', () => {
  it('1024 steps, one decimal under 10', () => {
    expect(bytes(0)).toBe('0 B');
    expect(bytes(1023)).toBe('1023 B');
    expect(bytes(1536)).toBe('1.5 KB');
    expect(bytes(12 * 1024 ** 2)).toBe('12 MB');
    expect(bytes(2.14 * 1024 ** 3)).toBe('2.1 GB');
    expect(bytes(10240 * 1024 ** 2)).toBe('10 GB');
    expect(bytes(9.99 * 1024 ** 2)).toBe('10 MB');
    expect(bytes(3 * 1024 ** 4)).toBe('3 TB');
  });
});
