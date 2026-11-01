import type { InviteInfo } from '../../lib/gen/InviteInfo';
import type { Role } from '../../lib/gen/Role';
import { relTime } from '../../lib/ui/messages';

export const ROLE_NAMES: Record<Role, string> = { member: 'Member', moderator: 'Moderator', owner: 'Owner' };
export const EXPIRES = [
  { hours: 1, label: '1 hour' },
  { hours: 24, label: '1 day' },
  { hours: 168, label: '7 days' },
  { hours: 720, label: '30 days' },
] as const;

export type InviteStatus = 'active' | 'used' | 'expired' | 'revoked';
export const STATUS_NAMES: Record<InviteStatus, string> = { active: 'Active', used: 'Used up', expired: 'Expired', revoked: 'Revoked' };

/** Revoked wins over used up, which wins over expired, as the server checks a code in that order of finality. */
export function inviteStatus(i: InviteInfo, now: number): InviteStatus {
  if (i.revoked) return 'revoked';
  if (i.uses >= i.max_uses) return 'used';
  if (i.expires_at <= now) return 'expired';
  return 'active';
}

/** When `ms` is: "in 7 days", "in 3 h", "in 20 min", "in less than a minute"; past times as relTime ("2 days ago"). */
export function untilText(ms: number, now: number): string {
  if (ms <= now) return relTime(ms, now);
  const min = Math.floor((ms - now) / 60_000);
  if (min < 1) return 'in less than a minute';
  if (min < 60) return `in ${min} min`;
  if (min < 24 * 60) return `in ${Math.floor(min / 60)} h`;
  const days = Math.floor(min / (24 * 60));
  return `in ${days} ${days === 1 ? 'day' : 'days'}`;
}

const RANK: Record<Role, number> = { owner: 0, moderator: 1, member: 2 };
/** Owner, then moderators, then members; by name inside each. */
export const byRole = <T extends { role: Role; display_name: string }>(a: T, b: T) =>
  RANK[a.role] - RANK[b.role] || a.display_name.localeCompare(b.display_name);

/** `ids` with `id` swapped with its neighbour above (-1) or below (1); unchanged at either end. */
export function moveRoom(ids: number[], id: number, by: -1 | 1): number[] {
  const i = ids.indexOf(id);
  const j = i + by;
  if (i < 0 || j < 0 || j >= ids.length) return ids;
  const out = [...ids];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];
/** Bytes in 1024 steps, as the quota is set in MiB (10240 shows as 10 GB): "512 MB", "2.1 GB", one decimal under 10. */
export function bytes(n: number): string {
  let u = 0;
  while (n >= 1024 && u < UNITS.length - 1) [n, u] = [n / 1024, u + 1];
  return `${u && n < 10 ? Number(n.toFixed(1)) : Math.round(n)} ${UNITS[u]}`;
}
