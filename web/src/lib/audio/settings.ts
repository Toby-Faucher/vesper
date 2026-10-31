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

export function clampSettings(s: Partial<AudioSettings>): AudioSettings {
  return {
    ...D,
    ...s,
    mode: (['deepfilter', 'browser', 'off'] as unknown[]).includes(s.mode) ? s.mode! : D.mode,
    attenDb: num(s.attenDb, D.attenDb, 6, 100),
    postFilter: num(s.postFilter, D.postFilter, 0, 0.05),
    inputBoostDb: num(s.inputBoostDb, D.inputBoostDb, 0, 20),
    bitrateKbps: num(s.bitrateKbps, D.bitrateKbps, 32, 510),
    echoCancellation: true,
    vad: { ...D.vad, ...s.vad },
    pushToTalk: { ...D.pushToTalk, ...s.pushToTalk },
  };
}
