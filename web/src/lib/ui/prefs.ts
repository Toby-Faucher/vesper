// The notification and call sound settings, apart from notify.ts so voice.svelte.ts can read them without importing
// notify.ts (which imports voice.svelte.ts).
import type { CuePrefs } from './cues';

export const NOTIFY_KEY = 'vesper.notify';
/**
 * `sounds` is the join and leave of others (its old meaning, so a saved choice carries over); `call` is the master switch
 * for every call sound except the ring (`ring` is the DM call ring's own switch, on unless turned off), `muteCues` and `connectCues` are my own mute, deafen and connection cues; `volume` is 0 to 100.
 */
export type NotifyPrefs = CuePrefs & { mentions: boolean; dms: boolean; sounds: boolean; volume: number; ring: boolean };
export const DEFAULTS: NotifyPrefs = { mentions: false, dms: false, sounds: false, call: true, muteCues: true, connectCues: true, volume: 50, ring: true };
const storage = (() => { try { return localStorage; } catch { return null; } })();

/** Read on every use, so a change in another tab applies here too. */
export function loadPrefs(): NotifyPrefs {
  try {
    const p = JSON.parse(storage?.getItem(NOTIFY_KEY) ?? '{}');
    return {
      mentions: p.mentions === true, dms: (p.dms ?? p.mentions) === true, // dms follows mentions until it is set (it used to share that switch)
      sounds: p.sounds === true, // off unless turned on
      call: p.call !== false, muteCues: p.muteCues !== false, connectCues: p.connectCues !== false, // on unless turned off
      ring: p.ring !== false,
      volume: Number.isFinite(p.volume) ? Math.min(100, Math.max(0, Math.round(p.volume))) : DEFAULTS.volume,
    };
  } catch {
    return { ...DEFAULTS };
  }
}
export function savePrefs(p: NotifyPrefs): void {
  try { storage?.setItem(NOTIFY_KEY, JSON.stringify(p)); } catch { /* storage blocked: the setting lasts this page */ }
}

/** "Even out voices" (Settings > Voice), off unless turned on. Its math is in level.ts. */
export const LEVEL_KEY = 'vesper.level';
export const loadLevelling = (): boolean => storage?.getItem(LEVEL_KEY) === 'on';
export function saveLevelling(on: boolean): void {
  try { storage?.setItem(LEVEL_KEY, on ? 'on' : 'off'); } catch { /* storage blocked: the setting lasts this page */ }
}
