import { describe, expect, it } from 'vitest';
import { DEFAULT_AUDIO_SETTINGS, clampSettings, type AudioSettings } from './settings';

describe('clampSettings', () => {
  it('fills defaults', () => {
    expect(clampSettings({})).toEqual(DEFAULT_AUDIO_SETTINGS);
    expect(DEFAULT_AUDIO_SETTINGS).toMatchObject({
      mode: 'deepfilter', attenDb: 100, postFilter: 0.02, inputBoostDb: 0, bitrateKbps: 64, dtx: true,
    });
  });
  it('clamps out-of-range values', () => {
    expect(clampSettings({ bitrateKbps: 10 }).bitrateKbps).toBe(32);
    expect(clampSettings({ bitrateKbps: 9999 }).bitrateKbps).toBe(510);
    const s = clampSettings({ attenDb: 1, postFilter: 1, inputBoostDb: -5 });
    expect([s.attenDb, s.postFilter, s.inputBoostDb]).toEqual([6, 0.05, 0]);
    expect(clampSettings({ attenDb: 500, inputBoostDb: 99 })).toMatchObject({ attenDb: 100, inputBoostDb: 12 });
  });
  it('echoCancellation cannot be set false', () => {
    expect(clampSettings({ echoCancellation: false } as unknown as Partial<AudioSettings>).echoCancellation).toBe(true);
  });
  it('replaces garbage from storage with defaults and does not share nested objects', () => {
    const s = clampSettings({ attenDb: NaN, mode: 'loud' } as unknown as Partial<AudioSettings>);
    expect(s.attenDb).toBe(100);
    expect(s.mode).toBe('deepfilter');
    s.vad.thresholdDb = 0;
    expect(DEFAULT_AUDIO_SETTINGS.vad.thresholdDb).not.toBe(0);
  });
  it('rejects wrong types and unknown keys from storage', () => {
    const s = clampSettings({
      dtx: 'yes', noiseSuppression: 1, autoGain: null, micId: 7, speakerId: 'spk',
      vad: { thresholdDb: 'x' }, pushToTalk: { enabled: 'true', key: 5 }, junk: 1,
    } as unknown as Partial<AudioSettings>);
    expect(s).toEqual({ ...DEFAULT_AUDIO_SETTINGS, speakerId: 'spk' });
    expect(clampSettings({ vad: { thresholdDb: -500 } }).vad.thresholdDb).toBe(-100);
  });
});
