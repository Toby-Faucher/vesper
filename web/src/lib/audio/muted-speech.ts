// "You are muted": speech heard on the mic while you are muted. The idea is Mumble's (it cues talking while muted);
// this is our own code. Nothing is sent while muted: the check reads a level on this device and only shows a notice.
// The desktop app has the same rule and numbers (`MutedSpeech` in desktop/src/voice/mix.rs).

/** The level that counts as speech: the speaking ring's (`isSpeaking` in volume.ts), as an RMS. */
export const SPEECH_RMS = 0.02;
/** Speech must add up to this before the notice shows. Each moment of quiet takes back as much, so clicks and short sounds never add up. */
export const SUSTAIN_MS = 500;
/** Nothing counts this long after you mute, so the end of a sentence you muted on does not show it. */
export const GRACE_MS = 1000;
/** The notice stays up this long, or until you unmute. Muting again does not bring it back. */
export const SHOW_MS = 5000;
/** After a notice, the next one waits this long, so it does not nag. */
export const COOLDOWN_MS = 60_000;
/** A longer gap between polls (a throttled timer) counts as this, so one loud poll cannot show it. */
const MAX_STEP_MS = 200;

export type MutedSpeech = {
  /** The last poll. */
  at: number | null;
  /** ms of speech heard, net of quiet. */
  heard: number;
  /** When listening began: the grace counts from it. */
  from: number | null;
  /** When the last notice showed: the cool down counts from it. Kept for the whole call. */
  shownAt: number | null;
  show: boolean;
};
export const MUTED_QUIET: MutedSpeech = { at: null, heard: 0, from: null, shownAt: null, show: false };

/**
 * One poll. `listening`: muted by hand (not by a moderator), not deafened, the setting on and the tab visible. `speech`:
 * the mic's level is above SPEECH_RMS now. Not listening hides the notice at once and starts the grace again.
 */
export function mutedSpeech(prev: MutedSpeech, now: number, o: { listening: boolean; speech: boolean }): MutedSpeech {
  if (!o.listening) return { ...prev, at: now, heard: 0, from: null, show: false };
  const from = prev.from ?? now;
  // Only the time since the last poll and after the grace counts.
  const step = prev.at === null ? 0 : Math.min(Math.max(now - Math.max(prev.at, from + GRACE_MS), 0), MAX_STEP_MS);
  let heard = o.speech ? Math.min(prev.heard + step, SUSTAIN_MS) : Math.max(0, prev.heard - step);
  let shownAt = prev.shownAt;
  if (heard >= SUSTAIN_MS && (shownAt === null || now - shownAt >= COOLDOWN_MS)) [shownAt, heard] = [now, 0];
  return { at: now, heard, from, shownAt, show: shownAt !== null && shownAt >= from && now - shownAt < SHOW_MS };
}

export const MUTED_SPEECH = 'You are muted. Unmute to be heard.';
