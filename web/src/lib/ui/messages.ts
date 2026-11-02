// Texts for client-side states. Errors from the server and devices have their own tables (api.ts, devices.ts, call.ts).
import { UI_MESSAGES as FIRST } from './messages-first';

// All of them, for the lazy pages. A text the first load needs goes in messages-first.ts.
export const UI_MESSAGES = {
  ...FIRST,
  composerOffline: 'You are offline. Your text is kept here, and Send works again once the connection is back.',
  tooLong: (over: number) => `Too long by ${over} ${over === 1 ? 'character' : 'characters'}.`,
  attachFailed: 'That file could not be attached. Upload it again.',
  dropFiles: 'Drop files to attach',
  accentFormat: (example: string) => `Six hex digits after #, like ${example}.`,
  accentOk: (ratio: number) => `Allowed. Dark text on it is ${ratio}:1.`,
  accentRefused: (what: string, ratio: number, need: number) => `Not allowed. ${what} is ${ratio}:1 and needs ${need}:1.`,
  accentInUse: 'Your custom colour is in use.',
  uploading: (n: number) => `Uploading ${n} ${n === 1 ? 'file' : 'files'}.`,
  filesReady: (n: number) => `${n} ${n === 1 ? 'file' : 'files'} ready to send.`,
  waitUploads: 'Waiting for uploads',
  imageFailed: 'The image did not load.',
  pictureType: 'Pick a PNG, JPEG, WebP or GIF picture.',
  pictureUnreadable: 'That picture could not be read. Try another file.',
  pictureRefused: 'The server did not take that picture. Try another file.',
  pictureTooLarge: 'That picture is too large. Try a smaller one.',
  pictureSaved: 'Picture saved.',
  pictureRemoved: 'Picture removed.',
  viewerFailed: 'Could not open the image. Check your connection and try again.',
  pinsFull: 'This room has 50 pins, the most it can hold. Unpin one first.',
  copyFailed: 'Could not copy. Select the text and copy it yourself.',
  adminDenied: 'You do not have access to admin pages.',
  adminOwnerOnly: 'Only the owner can open this page.',
  invitesLoading: 'Loading invites...',
  invitesFailed: 'Could not load the invites. Check your connection and try again.',
  invitesEmpty: 'No invites yet. Create one to bring a friend in.',
  inviteOnce: (n: number, until: string) => `This link is shown only now. It works for ${n} ${n === 1 ? 'person' : 'people'} until ${until}.`,
  inviteCopyFailed: 'Could not copy. Select the link and copy it.',
  inviteCopied: 'Link copied.',
  inviteModOnly: 'Only the owner can invite moderators.',
  membersLoading: 'Loading members...',
  membersFailed: 'Could not load the members. Check your connection and try again.',
  membersOwnerOnly: 'Only the owner can change roles and ban people.',
  banBody: 'They are signed out everywhere and cannot sign in.',
  resetBody: 'You get a link to send them. It works once, for 24 hours. When they use it, they set a new password and are signed out everywhere, and they are told who made the link. An older reset link for them stops working.',
  resetWrongPassword: 'Your password is not right.',
  resetOnce: (until: string) => `This link is shown only now. It works once, until ${until}.`,
  roomNameBad: 'Room names are 1 to 32 characters, with no hidden characters.',
  roomOrderChanged: 'The room list changed. Try again.',
  roomRestricted: 'Only listed people, moderators and the owner can see a restricted room.',
  roomNoDelete: 'Rooms cannot be deleted. Archive one to hide it from everyone.',
  roomLastOpen: 'The last room that is not archived cannot be archived.',
  roomArchiveHides: 'It leaves the room list, the switcher and search for everyone. Only this page still lists it.',
  roomArchiveKeeps: 'Its messages and files are kept. Unarchive brings the room back with all of them.',
  roomArchiveCall: 'Everyone in its voice call, you included, is taken out of it. Nobody can join it or post in it while it is archived.',
  roomMembersNone: 'Nobody matches that name.',
  diskLoading: 'Loading disk use...',
  diskFailed: 'Could not load the disk use. Check your connection and try again.',
  diskWarn: 'Uploads or the disk are over 80 % full. Delete old files or raise the quota (VESPER_UPLOAD_QUOTA_MB).',
  devicesLoading: 'Loading devices...',
  devicesFailed: 'Could not load your devices. Check your connection and try again.',
  devicesNone: 'No devices are signed in.',
  signOutFailed: 'Could not sign out that device. Try again.',
  passwordWrong: 'Your current password is not right.',
  passwordShort: 'Use 8 to 128 characters.',
  passwordMismatch: 'The two new passwords are not the same.',
  passwordChanged: 'Password changed. Your other devices were signed out.',
  profileSaved: 'Saved.',
} as const;

/** How long ago `ms` was, from `now`: "just now", "5 min ago", "3 h ago", "2 days ago", then a date after a week. */
export function relTime(ms: number, now: number): string {
  const min = Math.floor((now - ms) / 60_000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min} min ago`;
  if (min < 24 * 60) return `${Math.floor(min / 60)} h ago`;
  const days = Math.floor(min / (24 * 60));
  if (days < 7) return `${days} ${days === 1 ? 'day' : 'days'} ago`;
  return new Date(ms).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
}
