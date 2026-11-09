import { describe, expect, it } from 'vitest';
import { effectiveGain, isSpeaking, restoreVolumes, volumeToGain } from './volume';

const store = (o: Record<string, string>) => ({ getItem: (k: string) => o[k] ?? null }) as unknown as Storage;

describe('volumeToGain', () => {
  it('pins the curve', () => {
    expect(volumeToGain(0)).toBe(0);
    expect(volumeToGain(100)).toBe(1);
    expect(volumeToGain(50)).toBe(0.25);
    expect(20 * Math.log10(volumeToGain(250))).toBeCloseTo(10); // even dB steps above 100
    expect(volumeToGain(400)).toBeCloseTo(10); // +20 dB at the top
  });
  it('is monotonic', () => {
    for (let v = 1; v <= 400; v++) expect(volumeToGain(v)).toBeGreaterThan(volumeToGain(v - 1));
  });
  it('clamps', () => {
    expect(volumeToGain(-5)).toBe(0);
    expect(volumeToGain(999)).toBe(volumeToGain(400));
    expect(volumeToGain(NaN)).toBe(1);
  });
});
describe('restoreVolumes', () => {
  it('reads saved volumes', () => {
    const m = restoreVolumes(store({ 'vesper.volumes': '{"a":50,"b":150}' }));
    expect(m.get('a')).toBe(50);
    expect(m.get('b')).toBe(150);
  });
  it('ignores corrupt json, non-objects and bad values', () => {
    expect(restoreVolumes(store({ 'vesper.volumes': '{oops' })).size).toBe(0);
    expect(restoreVolumes(store({ 'vesper.volumes': '[1]' })).size).toBe(0);
    expect(restoreVolumes(store({ 'vesper.volumes': '{"a":"x","b":50}' })).size).toBe(1);
    expect(restoreVolumes(store({})).size).toBe(0);
  });
});
describe('effectiveGain / isSpeaking', () => {
  it('deafen silences', () => {
    expect(effectiveGain(100, true)).toBe(0);
    expect(effectiveGain(100, false)).toBe(1);
  });
  it('detects speech by rms', () => {
    expect(isSpeaking(new Float32Array(64))).toBe(false);
    expect(isSpeaking(new Float32Array(64).fill(0.3))).toBe(true);
  });
});
