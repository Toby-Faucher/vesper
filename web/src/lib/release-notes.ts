// What changed, for the "What's new" popup after an update. Written for the people who use Vesper, not for developers:
// one short line per change, nothing about the wire, migrations or the code. Each release adds its own entry here
// (docs/versions.md, "How a release is cut"). A lazy chunk with routes/WhatsNew.svelte.
import { newer } from './whats-new';

export type Note = { version: string; items: string[] };

/** The notes of the versions after `seen` up to `current`, newest first. Null when there is nothing to show: a first
 * visit (no `seen`), a bad stored value, the same version, a rollback, or no notes for those versions. */
export function whatsNew(seen: string | null, current: string, notes: readonly Note[]): Note[] | null {
  if (!newer(current, seen)) return null;
  const out = notes.filter((n) => newer(n.version, seen) && !newer(n.version, current));
  out.sort((a, b) => (newer(a.version, b.version) ? -1 : newer(b.version, a.version) ? 1 : 0));
  return out.length ? out : null;
}

export const RELEASE_NOTES: readonly Note[] = [
  {
    version: '1.8.3',
    items: [
      'A sound plays when someone joins or leaves your call. No sound? Turn on "Others joining and leaving" in Settings.',
      'Joining, leaving and losing a call sound louder, so you hear them.',
      'Moderators can stop a screen share from the person card.',
    ],
  },
  { version: '1.8.2', items: ["You'll see what's new here after each update."] },
  {
    version: '1.8.1',
    items: [
      'Videos open big in the viewer, like pictures.',
      'Older audio and video files in chat now play too.',
      'Files can be up to 100 MB.',
      'The room your mouse is heading for lights up a little.',
    ],
  },
  {
    version: '1.8.0',
    items: ['Audio and video play right in the chat.', 'Rooms start loading before you click them, so they open faster.'],
  },
];
