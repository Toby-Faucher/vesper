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
  vad: { thresholdDb: number };
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
  vad: { thresholdDb: -50 },
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
    inputBoostDb: num(s.inputBoostDb, D.inputBoostDb, 0, 20),
    bitrateKbps: num(s.bitrateKbps, D.bitrateKbps, 32, 510),
    dtx: bool(s.dtx, D.dtx),
    echoCancellation: true,
    noiseSuppression: bool(s.noiseSuppression, D.noiseSuppression),
    autoGain: bool(s.autoGain, D.autoGain),
    vad: { thresholdDb: num(vad.thresholdDb, D.vad.thresholdDb, -100, 0) },
    pushToTalk: { enabled: bool(ptt.enabled, D.pushToTalk.enabled), key: str(ptt.key, D.pushToTalk.key)! },
    micId: str(s.micId, D.micId),
    speakerId: str(s.speakerId, D.speakerId),
  };
}

export const SETTINGS_KEY = 'vesper.audio';

export function loadSettings(store: Storage): AudioSettings {
  try {
    const o = JSON.parse(store.getItem(SETTINGS_KEY) ?? 'null');
    return clampSettings(o && typeof o === 'object' ? o : {});
  } catch {
    return clampSettings({}); // corrupt storage: defaults
  }
}

export function saveSettings(store: Storage, s: AudioSettings): void {
  store.setItem(SETTINGS_KEY, JSON.stringify(s));
}
