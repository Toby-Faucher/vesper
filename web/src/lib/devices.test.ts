import { describe, expect, it, vi } from 'vitest';
import {
  applyBitrate, classifyMediaError, isBluetoothLabel, levelHint, missingDevice, newInputs, trackLive, watchDevices,
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
