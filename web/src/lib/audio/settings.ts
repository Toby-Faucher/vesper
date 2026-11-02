import type { DenoiseMode } from './mode';

export type AudioSettings = {
  mode: DenoiseMode;
  attenDb: number;
  postFilter: number;
  inputBoostDb: number;
  bitrateKbps: number;
  dtx: boolean;
  echoCancellation: true;
  noiseSuppression: boolean;
  autoGain: boolean;
  /** thresholdDb: the level marker, a sample peak in dBFS. gate: mute the denoised output below it (deepfilter only).
   * auto: the gate sets its own threshold from the noise floor instead of the marker (dynamics.ts). */
  vad: { thresholdDb: number; gate: boolean; auto: boolean };
  pushToTalk: { enabled: boolean; key: string };
  micId: string | null;
  speakerId: string | null;
};

export const DEFAULT_AUDIO_SETTINGS: AudioSettings = {
  mode: 'deepfilter',
  attenDb: 100,
  postFilter: 0.02,
  inputBoostDb: 0,
  bitrateKbps: 64,
  dtx: true,
  echoCancellation: true,
  noiseSuppression: false,
  autoGain: false,
  vad: { thresholdDb: -50, gate: false, auto: false }, // off by default: nobody's sound changes on upgrade
  pushToTalk: { enabled: false, key: 'Space' },
  micId: null,
  speakerId: null,
};

const D = DEFAULT_AUDIO_SETTINGS;
// Values may come from localStorage, so non-numbers fall back to the default.
const num = (v: unknown, d: number, lo: number, hi: number) =>
  typeof v === 'number' && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d;

const bool = (v: unknown, d: boolean) => (typeof v === 'boolean' ? v : d);
const str = (v: unknown, d: string | null) => (typeof v === 'string' ? v : d);

/** Builds every field explicitly, so wrong types and unknown keys from storage never get through. */
export function clampSettings(s: Partial<AudioSettings>): AudioSettings {
  const vad: Partial<AudioSettings['vad']> = s.vad ?? {};
  const ptt: Partial<AudioSettings['pushToTalk']> = s.pushToTalk ?? {};
  return {
    mode: (['deepfilter', 'browser', 'off'] as unknown[]).includes(s.mode) ? s.mode! : D.mode,
    attenDb: num(s.attenDb, D.attenDb, 6, 100),
    postFilter: num(s.postFilter, D.postFilter, 0, 0.05),
    inputBoostDb: num(s.inputBoostDb, D.inputBoostDb, 0, 12),
    bitrateKbps: num(s.bitrateKbps, D.bitrateKbps, 32, 510),
    dtx: bool(s.dtx, D.dtx),
    echoCancellation: true,
    noiseSuppression: bool(s.noiseSuppression, D.noiseSuppression),
    autoGain: bool(s.autoGain, D.autoGain),
    vad: { thresholdDb: num(vad.thresholdDb, D.vad.thresholdDb, -100, 0), gate: bool(vad.gate, D.vad.gate), auto: bool(vad.auto, D.vad.auto) },
    pushToTalk: { enabled: bool(ptt.enabled, D.pushToTalk.enabled), key: str(ptt.key, D.pushToTalk.key)! },
    micId: str(s.micId, D.micId),
    speakerId: str(s.speakerId, D.speakerId),
  };
}

export const SETTINGS_KEY = 'vesper.audio';

export function loadSettings(store: Storage | null): AudioSettings {
  try {
    const o = JSON.parse(store!.getItem(SETTINGS_KEY) ?? 'null');
    return clampSettings(o && typeof o === 'object' ? o : {});
  } catch {
    return clampSettings({}); // corrupt storage: defaults
  }
}

export function saveSettings(store: Storage | null, s: AudioSettings): void {
  try {
    store?.setItem(SETTINGS_KEY, JSON.stringify(s));
  } catch {
    /* blocked or full storage: settings just won't persist */
  }
}
