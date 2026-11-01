import { DEFAULT_AUDIO_SETTINGS as D, type AudioSettings } from '../../lib/audio/settings';

// The sections of Settings > Voice. Headings differ from the labels of the controls under them, so a name never reads twice.
export type Section = 'input' | 'noise' | 'output' | 'quality' | 'ptt';
export const NAMES: Record<Section, string> = { input: 'Input', noise: 'Noise and echo', output: 'Output', quality: 'Quality', ptt: 'Talk mode' };
// What "Reset to defaults" sets. It keeps the chosen microphone and speaker: a choice of device, not a setting with a
// default. So Output has no reset.
export const RESETS: Partial<Record<Section, Partial<AudioSettings>>> = {
  input: { vad: D.vad },
  noise: { mode: D.mode, attenDb: D.attenDb, postFilter: D.postFilter, inputBoostDb: D.inputBoostDb, noiseSuppression: D.noiseSuppression, autoGain: D.autoGain },
  quality: { bitrateKbps: D.bitrateKbps, dtx: D.dtx },
  ptt: { pushToTalk: D.pushToTalk },
};
