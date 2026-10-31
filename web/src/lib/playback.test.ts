import { afterEach, describe, expect, it, vi } from 'vitest';
import { Playback } from './playback';

const node = () => ({ connect: vi.fn(function (this: any, n: any) { return n; }), disconnect: vi.fn() });
function fakeCtx() {
  const gains: any[] = [];
  const ctx = {
    destination: {},
    resume: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    createMediaStreamSource: () => node(),
    createAnalyser: () => ({ fftSize: 0 }),
    createGain: () => { const g = { ...node(), gain: { value: 1 } }; gains.push(g); return g; },
  };
  return { ctx: ctx as unknown as AudioContext, gains };
}
class FakeAudio { muted = false; srcObject: unknown = {}; pause = vi.fn(); play = async () => {}; }
afterEach(() => vi.unstubAllGlobals());
const setup = (vols = new Map<string, number>()) => {
  vi.stubGlobal('Audio', FakeAudio);
  const f = fakeCtx();
  const p = new Playback(vols, f.ctx);
  return { ...f, p };
};

describe('Playback', () => {
  it('resumes the context on construction', () => {
    expect(setup().ctx.resume).toHaveBeenCalled();
  });
  it('deafen silences every gain and undeafen restores saved volumes', () => {
    const { p, gains } = setup(new Map([['a', 50]]));
    p.addRemote('a', {} as MediaStream);
    p.addRemote('b', {} as MediaStream);
    expect(gains.map((g) => g.gain.value)).toEqual([0.25, 1]);
    p.setDeafened(true);
    expect(gains.map((g) => g.gain.value)).toEqual([0, 0]);
    p.setDeafened(false);
    expect(gains.map((g) => g.gain.value)).toEqual([0.25, 1]);
  });
  it('remove disconnects nodes; unknown id is a no-op', () => {
    const { p, gains } = setup();
    p.addRemote('a', {} as MediaStream);
    p.remove('a');
    expect(gains[0].disconnect).toHaveBeenCalled();
    expect(() => p.remove('zzz')).not.toThrow();
    p.remove('a'); // idempotent
  });
});
