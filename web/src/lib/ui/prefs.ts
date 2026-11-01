// The notification and call sound settings, apart from notify.ts so voice.svelte.ts can read them without importing
// notify.ts (which imports voice.svelte.ts).
import type { CuePrefs } from './cues';

export const NOTIFY_KEY = 'vesper.notify';
/**
 * `sounds` is the join and leave of others (its old meaning, so a saved choice carries over); `call` is the master switch
 * for every call sound, `muteCues` and `connectCues` are my own mute, deafen and connection cues; `volume` is 0 to 100.
 */
export type NotifyPrefs = CuePrefs & { mentions: boolean; sounds: boolean; volume: number };
export const DEFAULTS: NotifyPrefs = { mentions: false, sounds: false, call: true, muteCues: true, connectCues: true, volume: 50 };
const storage = (() => { try { return localStorage; } catch { return null; } })();

/** Read on every use, so a change in another tab applies here too. */
export function loadPrefs(): NotifyPrefs {
  try {
    const p = JSON.parse(storage?.getItem(NOTIFY_KEY) ?? '{}');
    return {
      mentions: p.mentions === true, sounds: p.sounds === true, // off unless turned on
      call: p.call !== false, muteCues: p.muteCues !== false, connectCues: p.connectCues !== false, // on unless turned off
      volume: Number.isFinite(p.volume) ? Math.min(100, Math.max(0, Math.round(p.volume))) : DEFAULTS.volume,
    };
  } catch {
    return { ...DEFAULTS };
  }
}
export function savePrefs(p: NotifyPrefs): void {
  try { storage?.setItem(NOTIFY_KEY, JSON.stringify(p)); } catch { /* storage blocked: the setting lasts this page */ }
}
