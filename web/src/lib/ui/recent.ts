// The rooms you opened last, most recent first, per account: localStorage['vesper.recent.<user id>']. The switcher ranks by it.
export const MAX_RECENT = 10;
export const recentKey = (userId: string) => `vesper.recent.${userId}`;
type Store = Pick<Storage, 'getItem' | 'setItem'> | null;

export function loadRecent(storage: Store, userId: string): number[] {
  try {
    const v: unknown = JSON.parse(storage?.getItem(recentKey(userId)) ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is number => Number.isSafeInteger(x)).slice(0, MAX_RECENT) : [];
  } catch {
    return [];
  }
}

/** Puts `room` first and keeps MAX_RECENT. Returns the new list. */
export function pushRecent(storage: Store, userId: string, room: number): number[] {
  const list = [room, ...loadRecent(storage, userId).filter((x) => x !== room)].slice(0, MAX_RECENT);
  try { storage?.setItem(recentKey(userId), JSON.stringify(list)); } catch { /* storage full or blocked */ }
  return list;
}
