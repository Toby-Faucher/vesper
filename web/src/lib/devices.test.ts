import { describe, expect, it, vi } from 'vitest';
import {
  applyBitrate, classifyMediaError, hintLevelDb, micStartError, isBluetoothLabel, levelHint, missingDevice, newInputs, trackLive, watchDevices,
} from './devices';
import { loadSettings, saveSettings, SETTINGS_KEY } from './audio/settings';
import { DEFAULT_AUDIO_SETTINGS } from './audio/settings';

const err = (name: string) => Object.assign(new Error('x'), { name });
const dev = (deviceId: string, kind = 'audioinput', label = deviceId) => ({ deviceId, kind, label }) as MediaDeviceInfo;

describe('classifyMediaError', () => {
  it('maps known DOMException names', () => {
    expect(classifyMediaError(err('NotAllowedError'))).toBe('denied');
    expect(classifyMediaError(err('NotFoundError'))).toBe('no-device');
    expect(classifyMediaError(err('NotReadableError'))).toBe('in-use');
  });
  it('falls back to unknown', () => {
    expect(classifyMediaError(err('AbortError'))).toBe('unknown');
    expect(classifyMediaError(null)).toBe('unknown');
    expect(classifyMediaError('boom')).toBe('unknown');
  });
  it('reads the error name out of a capture "microphone lost" reason', () => {
    expect(classifyMediaError('microphone lost: NotAllowedError: Permission denied')).toBe('denied');
    expect(classifyMediaError('microphone lost: ended')).toBe('ended');
    expect(classifyMediaError('microphone lost: muted by the system')).toBe('muted');
  });
});

describe('micStartError', () => {
  // [error, start of the sentence, details]
  const cases: [unknown, string, string][] = [
    [{ name: 'OverconstrainedError', message: 'Constraints could not be satisfied.' }, 'Vesper could not find a microphone.', 'OverconstrainedError: Constraints could not be satisfied.'],
    [new DOMException('Requested device not found', 'NotFoundError'), 'Vesper could not find a microphone.', 'NotFoundError: Requested device not found'],
    [new DOMException('Permission denied', 'NotAllowedError'), 'Microphone access was denied.', 'NotAllowedError: Permission denied'],
    [new DOMException('', 'NotReadableError'), 'The microphone is in use by another app.', 'NotReadableError'],
    [new DOMException('Starting audioinput failed', 'AbortError'), 'Vesper could not start the microphone.', 'AbortError: Starting audioinput failed'],
    ['boom', 'Vesper could not start the microphone.', 'boom'],
  ];
  it.each(cases)('%o', (e, sentence, details) => {
    const m = micStartError(e);
    expect(m.startsWith(sentence)).toBe(true);
    expect(m.endsWith(` Details: ${details}`)).toBe(true);
    expect(m).not.toMatch(/call stays connected/);
  });
  it('names the way out', () => {
    for (const n of ['NotFoundError', 'NotReadableError', 'AbortError']) expect(micStartError({ name: n })).toContain('Settings > Voice > Microphone');
  });
});

describe('isBluetoothLabel', () => {
  it.each(['AirPods Pro', 'Bluetooth Headset', 'WH-1000XM4 Hands-Free AG Audio', 'Galaxy Buds', 'Buds2 Pro', 'Phone HFP'])(
    'flags %s', (l) => expect(isBluetoothLabel(l)).toBe(true));
  it.each(['Headset Microphone', 'Sennheiser Momentum 4 Wired', 'Blue Yeti', 'Built-in Audio Analog Stereo', ''])('ignores %s', (l) => expect(isBluetoothLabel(l)).toBe(false));
});

describe('levelHint', () => {
  it('flags clipping on a near-full-scale peak', () => expect(levelHint([-30, -0.5])).toBe('clipping'));
  it('flags too quiet when the whole window is low', () => expect(levelHint([-60, -50, -45])).toBe('too-quiet'));
  it('is fine otherwise, and silent without data', () => {
    expect(levelHint([-60, -20])).toBe('ok');
    expect(levelHint([])).toBe('ok');
  });
});

describe('hintLevelDb', () => {
  const stats = (inputPeak: number, ageMs: number | null) =>
    ({ pipeline: { inputPeak }, ageMs, mic: {}, context: {} }) as unknown as Parameters<typeof hintLevelDb>[1];
  it('uses the pre-limiter peak in deepfilter mode, so a clipping mic still reads clipping', () => {
    // The sent track is limited to under -1 dBFS (meter -1.5), the mic before it hits full scale.
    expect(levelHint([hintLevelDb(-1.5, stats(1, 200))])).toBe('clipping');
    expect(hintLevelDb(-100, stats(0.1, 200))).toBeCloseTo(-20, 9); // a gated silence is not a dead mic
    expect(hintLevelDb(-30, stats(0, 200))).toBe(-100);
  });
  it('falls back to the meter without a fresh worklet report', () => {
    expect(hintLevelDb(-0.5, null)).toBe(-0.5);
    expect(hintLevelDb(-12, stats(1, 5000))).toBe(-12);
    expect(hintLevelDb(-12, stats(1, null))).toBe(-12);
  });
});

describe('device lists', () => {
  it('detects a selected mic that vanished', () => {
    expect(missingDevice([dev('a')], 'b')).toBe(true);
    expect(missingDevice([dev('a')], 'a')).toBe(false);
    expect(missingDevice([dev('a')], null)).toBe(false);
  });
  it('finds newly appeared inputs only', () => {
    expect(newInputs([dev('a')], [dev('a'), dev('b'), dev('s', 'audiooutput')]).map((d) => d.deviceId)).toEqual(['b']);
  });
  it('does not offer anything from a pre-permission (unlabelled) enumeration', () => {
    expect(newInputs([dev('', 'audioinput', '')], [dev('a'), dev('b')])).toEqual([]);
    expect(newInputs([dev('a')], [dev('a'), dev('', 'audioinput', '')])).toEqual([]);
  });
  it('watchDevices re-enumerates on devicechange and unsubscribes', async () => {
    const t = new EventTarget();
    const list = [dev('a')];
    const md = Object.assign(t, { enumerateDevices: async () => list }) as unknown as MediaDevices;
    const cb = vi.fn();
    const off = watchDevices(cb, md);
    t.dispatchEvent(new Event('devicechange'));
    await new Promise((r) => setTimeout(r, 0));
    expect(cb).toHaveBeenCalledWith(list);
    off();
    t.dispatchEvent(new Event('devicechange'));
    await new Promise((r) => setTimeout(r, 0));
    expect(cb).toHaveBeenCalledTimes(1);
  });
});

describe('trackLive', () => {
  it('mute always wins', () => {
    expect(trackLive(true, false, false)).toBe(false);
    expect(trackLive(true, true, true)).toBe(false);
  });
  it('without PTT live iff not muted', () => expect(trackLive(false, false, false)).toBe(true));
  it('with PTT live only while held', () => {
    expect(trackLive(false, true, false)).toBe(false);
    expect(trackLive(false, true, true)).toBe(true);
  });
});

describe('applyBitrate', () => {
  it('sets maxBitrate in bps on every encoding', async () => {
    const params = { encodings: [{}] } as RTCRtpSendParameters;
    const sender = { getParameters: () => params, setParameters: vi.fn(async () => {}) };
    await applyBitrate(sender as unknown as RTCRtpSender, 96);
    expect(sender.setParameters).toHaveBeenCalledWith({ encodings: [{ maxBitrate: 96000 }] });
  });
  it('creates an encoding when the browser reports none', async () => {
    const sender = { getParameters: () => ({}), setParameters: vi.fn(async () => {}) };
    await applyBitrate(sender as unknown as RTCRtpSender, 32);
    expect(sender.setParameters).toHaveBeenCalledWith({ encodings: [{ maxBitrate: 32000 }] });
  });
});

describe('settings persistence', () => {
  const store = (v?: string) => {
    const m = new Map<string, string>(v === undefined ? [] : [[SETTINGS_KEY, v]]);
    return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, x: string) => void m.set(k, x) } as Storage;
  };
  it('round-trips', () => {
    const s = store();
    saveSettings(s, { ...DEFAULT_AUDIO_SETTINGS, bitrateKbps: 128 });
    expect(loadSettings(s).bitrateKbps).toBe(128);
  });
  it('corrupt or missing storage gives defaults', () => {
    expect(loadSettings(store('{nope'))).toEqual(DEFAULT_AUDIO_SETTINGS);
    expect(loadSettings(store())).toEqual(DEFAULT_AUDIO_SETTINGS);
    expect(loadSettings(store('42'))).toEqual(DEFAULT_AUDIO_SETTINGS);
  });
  it('clamps out-of-range stored values', () => {
    expect(loadSettings(store('{"bitrateKbps":9999}')).bitrateKbps).toBe(510);
  });
});
