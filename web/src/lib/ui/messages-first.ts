// The texts the first load needs (the shell, the sidebar, the connection banner, sign in, uploads). The rest are in
// messages.ts, which the lazy pages import: one table cost 1.5 kB gzip of first load (measured 2026-11-01).
export const UI_MESSAGES = {
  connecting: 'Connecting to the server...',
  reconnecting: (s: number) => `Connection lost. Trying again in ${s} s.`,
  reconnectingNow: 'Connection lost. Trying again now.',
  reconnectingSr: 'Reconnecting to the server.', // read once per drop; the countdown above is not read
  backOnline: 'Back online.',
  retryNow: 'Try now',
  reconnect: 'Reconnect',
  updated: 'Vesper was updated. Reload to continue.',
  reload: 'Reload',
  noRooms: 'No rooms yet.',
  noRoomsBody: 'The owner can add one in the admin pages.',
  roomMissing: 'This room is not here.',
  roomMissingBody: 'It was removed, or you do not have access to it.',
  pageMissing: 'There is no page at this address.',
  goHome: 'Go to your rooms',
  openFirstRoom: (name: string) => `Open ${name}`,
  chatLoading: 'Loading messages...',
  chatFailed: 'The chat did not load. Reload the page to try again.',
  switcherLoading: 'Loading...',
  switcherFailed: 'Could not load the switcher. Check your connection and try again.',
  searchFailed: 'Could not load search. Check your connection and try again.',
  offlineAction: 'You are offline. This works again once the connection is back.',
  fileTooLarge: (bytes: number) => `Files can be up to ${Math.floor(bytes / 1024 / 1024)} MB.`,
  uploadQuota: 'The server has no room for more files. Ask the owner to free some space.',
  uploadRate: 'Too many uploads in a short time. Wait a minute and try again.',
  uploadNetwork: 'The upload stopped. Check your connection and try again.',
  pinsFailed: 'The pins did not load.',
  // Display names are not unique, so the unique @username goes with it: a taker can not pose as the owner.
  resetNotice: (by: string | null, username: string | null, when: string) =>
    `Your password was reset ${by === null ? 'from the server command line' : `by ${by} (@${username})`} on ${when}.`,
} as const;
