// Which call sound a change of my own call state plays. Pure, so the table of transitions is tested on its own.
import type { Peer } from '../gen/Peer';

export type Cue = 'join' | 'leave' | 'mute' | 'unmute' | 'deafen' | 'undeafen' | 'connect' | 'disconnect' | 'lost' | 'ring' | 'share-start' | 'share-stop';
/** `online` is the socket: a call that ends while it is down was lost, not left. */
export type CallSnap = { joined: boolean; muted: boolean; deafened: boolean; online: boolean };
export type CuePrefs = { call: boolean; muteCues: boolean; connectCues: boolean };

/** `prev` is null on the first look (page load): nothing plays for a state that was already there. */
export function callCue(prev: CallSnap | null, next: CallSnap, p: CuePrefs): Cue | null {
  if (!prev || !p.call) return null;
  // Joining or leaving resets the flags in the same step: only the connection cue plays then.
  if (prev.joined !== next.joined) {
    if (!p.connectCues) return null;
    return next.joined ? 'connect' : next.online ? 'disconnect' : 'lost';
  }
  if (!next.joined || !p.muteCues) return null;
  if (prev.deafened !== next.deafened) return next.deafened ? 'deafen' : 'undeafen';
  if (next.deafened) return null; // deafened: only the deafen cue itself
  if (prev.muted !== next.muted) return next.muted ? 'mute' : 'unmute';
  return null;
}

/** Someone in my call starts or stops sharing. A joiner who is already sharing, and a change of settings, play nothing. */
export function shareCue(prev: Peer[], next: Peer[], roomId: number): Cue | null {
  const was = new Map(prev.filter((p) => p.room_id === roomId).map((p) => [p.user_id, !!p.share]));
  for (const p of next) {
    const before = was.get(p.user_id);
    if (p.room_id === roomId && before !== undefined && before !== !!p.share) return p.share ? 'share-start' : 'share-stop';
  }
  return null;
}
