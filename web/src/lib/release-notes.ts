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
    version: '2.4.0',
    items: [
      'Talk while muted and the call bar says "You are muted". Turn it off in Settings.',
    ],
  },
  {
    version: '2.3.2',
    items: [
      'Calls ride out a short network drop: nobody leaves and voice keeps going.',
      'Desktop: sending a screen share is much faster on large screens.',
      'Desktop: clicks work again after a dragged file goes nowhere.',
      'Desktop: the window buttons no longer cover a full screen share.',
    ],
  },
  {
    version: '2.3.1',
    items: [
      'Desktop: drag across many messages to select them, and Ctrl+A selects the chat.',
      'Desktop: quotes, times, file names, pins and search results can be selected.',
      'Windows: notifications say Vesper, and the taskbar icon shows unread messages.',
    ],
  },
  {
    version: '2.3.0',
    items: [
      'The desktop app updates itself when you click Update and restart.',
      'A Windows installer, and an AppImage for Linux.',
      'Desktop notifications, and an unread dot on the tray icon.',
      'Start in the tray, choose what closing does, and set mute and deafen keys.',
      'Pick a screen or window to share on Windows and X11.',
      'Drop files on the chat, and pop a share out into its own window.',
      'Your share reads "Sharing" while nobody watches.',
    ],
  },
  {
    version: '2.2.0',
    items: [
      'Share your screen from the desktop app, on Windows and Linux.',
      'Shares in the desktop app play smoothly, fit the window and tile like here.',
      'New: @everyone pings the room, @here pings who is online.',
      'Files sent without text no longer show their names as a message.',
      'Failed uploads try again by themselves.',
      'A new share is lowered to fit, instead of "too many screens".',
      'RNNoise is a new noise cancellation choice.',
      'Selected text uses your accent colour.',
    ],
  },
  {
    version: '2.1.0',
    items: [
      'Vesper now has a desktop app for Windows and Linux.',
      'Get it in Settings, Desktop app. Push to talk works while you play.',
    ],
  },
  {
    version: '1.8.3',
    items: [
      'A sound plays when someone joins or leaves your call.',
      'No sound? Turn it on in Settings, Notifications.',
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
