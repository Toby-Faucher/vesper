// The sharer's side: the screen capture and the video sender's encoding. In the call chunk (voice.svelte.ts gets it there).
import type { Prefer } from './prefs';

type Settings = { fps: 30 | 60; kbps: number; prefer: Prefer };

/** One full-size layer. Voice gets the higher priority (join.ts). */
export function encodingFor(s: { fps: 30 | 60; kbps: number }, active: boolean): RTCRtpEncodingParameters {
  return { active, maxBitrate: s.kbps * 1000, maxFramerate: s.fps, scaleResolutionDownBy: 1, priority: 'low', networkPriority: 'low' };
}
export const degradationFor = (p: Prefer): RTCDegradationPreference => (p === 'sharp' ? 'maintain-resolution' : 'maintain-framerate');
export const hintFor = (p: Prefer): 'detail' | 'motion' => (p === 'sharp' ? 'detail' : 'motion');

/** `active` false stops the encoder: nobody watches. Also sets the sending track's content hint, for a changed preference. */
export async function applyShare(sender: RTCRtpSender, s: Settings, active: boolean): Promise<void> {
  const t = sender.track;
  if (t && 'contentHint' in t) t.contentHint = hintFor(s.prefer);
  const set = (withPref: boolean) => {
    const p = sender.getParameters();
    p.encodings = [{ ...p.encodings[0], ...encodingFor(s, active) }];
    return sender.setParameters(withPref ? { ...p, degradationPreference: degradationFor(s.prefer) } : p);
  };
  // A browser without degradationPreference may refuse the whole call: the encoding matters more.
  await set(true).catch(() => set(false));
}

/** The browser's screen picker. Null when the person cancels it. */
export async function capture(fps: 30 | 60, prefer: Prefer): Promise<MediaStreamTrack | null> {
  let s: MediaStream;
  try {
    s = await navigator.mediaDevices.getDisplayMedia({ video: { frameRate: fps }, audio: false });
  } catch (e) {
    const name = (e as { name?: string } | null)?.name;
    if (name === 'NotAllowedError' || name === 'AbortError') return null;
    throw e;
  }
  const t = s.getVideoTracks()[0];
  if ('contentHint' in t) t.contentHint = hintFor(prefer);
  return t;
}
