// Texts for client-side states. Errors from the server and devices have their own tables (api.ts, devices.ts, call.ts).
export const UI_MESSAGES = {
  connecting: 'Connecting to the server...',
  reconnecting: (s: number) => `Connection lost. Trying again in ${s} s.`,
  reconnectingNow: 'Connection lost. Trying again now.',
  reconnectingSr: 'Reconnecting to the server.', // read once per drop; the countdown above is not read
  backOnline: 'Back online.',
  retryNow: 'Try now',
  reconnect: 'Reconnect',
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
  adminLater: 'The admin pages are not built yet.',
  sectionLater: 'Nothing to set here yet.',
} as const;
