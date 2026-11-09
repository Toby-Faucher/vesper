import { afterEach, describe, expect, it, vi } from 'vitest';
import { Playback } from './playback';

const node = () => ({ connect: vi.fn(function (this: any, n: any) { return n; }), disconnect: vi.fn() });
function fakeCtx() {
  const gains: any[] = [];
  const limits: any[] = [];
  const ctx = {
    destination: {},
    resume: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    createMediaStreamSource: () => node(),
    currentTime: 0,
    amp: 0, // the peak of the sine every analyser reads
    createAnalyser: () => ({ fftSize: 0, getFloatTimeDomainData: (b: Float32Array) => b.forEach((_, i) => (b[i] = ctx.amp * Math.sin(i / 3))) }),
    createDynamicsCompressor: () => {
      const p = () => ({ value: 0 });
      const c = { ...node(), threshold: p(), knee: p(), ratio: p(), attack: p(), release: p() };
      limits.push(c);
      return c;
    },
    createGain: () => {
      const g = { ...node(), gain: { value: 1, setTargetAtTime: vi.fn((v: number) => (g.gain.value = v)), cancelScheduledValues: vi.fn() } };
      gains.push(g);
      return g;
    },
  };
  return { ctx: ctx as unknown as AudioContext & { amp: number; currentTime: number }, gains, limits };
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
    const vol = () => gains.filter((_, i) => i % 2 === 0).map((g) => g.gain.value); // each remote: volume, then level
    expect(vol()).toEqual([0.25, 1]);
    p.setDeafened(true);
    expect(vol()).toEqual([0, 0]);
    p.setDeafened(false);
    expect(vol()).toEqual([0.25, 1]);
  });
  it('the volume feeds a limiter, and only the limiter reaches the speakers', () => {
    const { p, gains, limits, ctx } = setup();
    p.addRemote('a', {} as MediaStream);
    expect(gains[0].connect).toHaveBeenCalledWith(limits[0]);
    expect(limits[0].connect).toHaveBeenCalledWith(ctx.destination);
    expect(gains[0].connect).not.toHaveBeenCalledWith(ctx.destination);
    expect(limits[0].threshold.value).toBe(-3);
    p.remove('a');
    expect(limits[0].disconnect).toHaveBeenCalled();
  });
  it('remove disconnects nodes; unknown id is a no-op', () => {
    const { p, gains } = setup();
    p.addRemote('a', {} as MediaStream);
    p.remove('a');
    expect(gains[0].disconnect).toHaveBeenCalled();
    expect(() => p.remove('zzz')).not.toThrow();
    p.remove('a'); // idempotent
  });
  it('levelling lifts a quiet talker under the manual volume, only when on, and off resets it', async () => {
    const { p, gains, ctx } = setup(new Map([['a', 50]]));
    p.addLocal('me', {} as MediaStream);
    p.addRemote('a', {} as MediaStream);
    const [vol, level] = gains;
    const talk = (s: number) => { for (let i = 0; i < s * 10; i++) { ctx.currentTime += 0.1; p.speaking(); } };
    ctx.amp = 0.02; // about -37 dBFS RMS: quiet, above the floor
    talk(5);
    expect(level.gain.value).toBe(1); // off by default
    p.setLevelling(true);
    await vi.dynamicImportSettled(); // the math loads when levelling is first turned on
    talk(20);
    expect(level.gain.value).toBeGreaterThan(2); // lifted by more than 6 dB
    expect(p.levelDb('a')).toBeLessThanOrEqual(10);
    expect(vol.gain.value).toBe(0.25); // the manual volume is untouched
    expect(p.levelDb('me')).toBeUndefined(); // the local mic is analysed only
    p.setLevelling(false);
    expect(level.gain.setTargetAtTime).toHaveBeenLastCalledWith(1, ctx.currentTime, 0.05); // ramped back, no jump
    expect(p.levelDb('a')).toBe(0);
  });
  it('levelling never lifts steady room noise under the floor', async () => {
    const { p, gains, ctx } = setup();
    p.addRemote('a', {} as MediaStream);
    p.setLevelling(true);
    await vi.dynamicImportSettled();
    ctx.amp = 0.008; // about -45 dBFS RMS
    for (let i = 0; i < 300; i++) { ctx.currentTime += 0.1; p.speaking(); }
    expect(p.levelDb('a')).toBe(0);
    expect(gains[1].gain.value).toBe(1);
  });
});
