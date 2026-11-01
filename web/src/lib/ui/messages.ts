// Texts for client-side states. Errors from the server and devices have their own tables (api.ts, devices.ts, call.ts).
export const UI_MESSAGES = {
  connecting: 'Connecting to the server...',
  reconnecting: (s: number) => `Connection lost. Trying again in ${s} s.`,
  reconnectingNow: 'Connection lost. Trying again now.',
  retryNow: 'Try now',
  reconnect: 'Reconnect',
  noRooms: 'No rooms yet.',
  noRoomsBody: 'The owner can add one in the admin pages.',
  roomMissing: 'This room is not here.',
  roomMissingBody: 'It was removed, or you do not have access to it.',
  pageMissing: 'There is no page at this address.',
  goHome: 'Go to your rooms',
  openFirstRoom: (name: string) => `Open ${name}`,
  adminLater: 'The admin pages are not built yet.',
  sectionLater: 'Nothing to set here yet.',
} as const;
