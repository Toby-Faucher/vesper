// The "What's new" version check, in the first load. The notes and the dialog are a lazy chunk (release-notes.ts,
// routes/WhatsNew.svelte), loaded only when the server is newer than the version this browser saw last.
export const SEEN_KEY = 'vesper.seenVersion';
const XYZ = /^\d+\.\d+\.\d+$/;

/** True when version `a` comes after `b`, by number (1.10.0 after 1.9.0). False when either is not x.y.z. */
export const newer = (a: string | null, b: string | null): boolean =>
  XYZ.test(a ?? '') && XYZ.test(b ?? '') && a!.localeCompare(b!, 'en', { numeric: true }) > 0;

/** Stores `current` as the seen version. Returns the one stored before when `current` is newer than it, else null: a
 * first visit, the same version, a rollback, a bad stored value, or blocked storage. */
export function takeSeen(storage: Pick<Storage, 'getItem' | 'setItem'> | null, current: string): string | null {
  try {
    const seen = storage!.getItem(SEEN_KEY);
    storage!.setItem(SEEN_KEY, current);
    return newer(current, seen) ? seen : null;
  } catch {
    return null; // storage blocked (or null): show nothing
  }
}
