import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyShare, capture, degradationFor, encodingFor, hintFor } from './send';

afterEach(() => vi.unstubAllGlobals());

describe('encodingFor', () => {
  it('sets the bitrate, frame rate and full size, low priority', () => {
    expect(encodingFor({ fps: 60, kbps: 8000 }, false)).toEqual({
      active: false, maxBitrate: 8_000_000, maxFramerate: 60, scaleResolutionDownBy: 1, priority: 'low', networkPriority: 'low',
    });
    expect(encodingFor({ fps: 30, kbps: 500 }, true)).toMatchObject({ active: true, maxBitrate: 500_000, maxFramerate: 30 });
  });
});

describe('sharp and smooth', () => {
  it('maps to a degradation preference and a content hint', () => {
    expect([degradationFor('sharp'), hintFor('sharp')]).toEqual(['maintain-resolution', 'detail']);
    expect([degradationFor('smooth'), hintFor('smooth')]).toEqual(['maintain-framerate', 'motion']);
  });
});

describe('applyShare', () => {
  const sender = (refuse: boolean) => {
    const set: RTCRtpSendParameters[] = [];
    const s = {
      set,
      track: { contentHint: '' },
      getParameters: () => ({ transactionId: 't', encodings: [{ rid: 'x' }] }),
      setParameters: vi.fn(async (p: RTCRtpSendParameters) => {
        if (refuse && p.degradationPreference) throw new DOMException('no', 'OperationError');
        set.push(p);
      }),
    };
    return s;
  };
  it('sets the encoding and the degradation preference', async () => {
    const s = sender(false);
    await applyShare(s as unknown as RTCRtpSender, { fps: 60, kbps: 8000, prefer: 'sharp' }, true);
    expect(s.set).toEqual([{ transactionId: 't', degradationPreference: 'maintain-resolution', encodings: [{ rid: 'x', ...encodingFor({ fps: 60, kbps: 8000 }, true) }] }]);
    expect(s.track.contentHint).toBe('detail');
  });
  it('a browser that refuses the degradation preference still gets the encoding', async () => {
    const s = sender(true);
    await expect(applyShare(s as unknown as RTCRtpSender, { fps: 30, kbps: 4000, prefer: 'smooth' }, false)).resolves.toBeUndefined();
    expect(s.setParameters).toHaveBeenCalledTimes(2);
    expect(s.set).toEqual([{ transactionId: 't', encodings: [{ rid: 'x', ...encodingFor({ fps: 30, kbps: 4000 }, false) }] }]);
    expect(s.track.contentHint).toBe('motion');
  });
});

describe('capture', () => {
  const display = (impl: () => Promise<unknown>) => {
    const getDisplayMedia = vi.fn(impl);
    vi.stubGlobal('navigator', { mediaDevices: { getDisplayMedia } });
    return getDisplayMedia;
  };
  for (const name of ['NotAllowedError', 'AbortError']) {
    it(`returns null when the picker is cancelled (${name})`, async () => {
      display(async () => { throw new DOMException('x', name); });
      expect(await capture(60, 'sharp')).toBeNull();
    });
  }
  it('throws other errors', async () => {
    display(async () => { throw new DOMException('x', 'NotReadableError'); });
    await expect(capture(60, 'sharp')).rejects.toThrow('x');
  });
  it('asks for video only at the frame rate, and sets the content hint where the track has one', async () => {
    const hinted = { contentHint: '' };
    const gdm = display(async () => ({ getVideoTracks: () => [hinted] }));
    expect(await capture(30, 'smooth')).toBe(hinted);
    expect(gdm).toHaveBeenCalledWith({ video: { frameRate: 30 }, audio: false });
    expect(hinted.contentHint).toBe('motion');
    const plain = {};
    display(async () => ({ getVideoTracks: () => [plain] }));
    expect(await capture(60, 'sharp')).toBe(plain);
    expect('contentHint' in plain).toBe(false);
  });
});
