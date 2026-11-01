import { describe, expect, it } from 'vitest';
import { DEFAULT_AUDIO_SETTINGS as D, type AudioSettings } from '../../lib/audio/settings';
import { RESETS } from './sections';

describe('Voice section resets', () => {
  it('keeps_the_chosen_mic_and_speaker', () => {
    const mine: AudioSettings = { ...D, micId: 'mic-2', speakerId: 'spk-3', vad: { thresholdDb: -20, gate: true }, autoGain: !D.autoGain, bitrateKbps: 128 };
    for (const [s, patch] of Object.entries(RESETS)) {
      const after = { ...mine, ...patch };
      expect([after.micId, after.speakerId], s).toEqual(['mic-2', 'spk-3']);
    }
    expect(RESETS.output).toBeUndefined();
  });
  it('sets_every_other_field_back', () => {
    const covered = new Set(Object.values(RESETS).flatMap((p) => Object.keys(p!)));
    expect([...covered].sort()).toEqual(Object.keys(D).filter((k) => !['micId', 'speakerId', 'echoCancellation'].includes(k)).sort()); // echo cancellation is always on
  });
});
