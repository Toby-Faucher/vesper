import { describe, expect, it } from 'vitest';
import { FrameRing } from './ring';

const block = (start: number, n = 128) => Float32Array.from({ length: n }, (_, i) => start + i);

describe('FrameRing', () => {
  it('4 pushes of 128 yield one 480 frame in order, 32 left', () => {
    const r = new FrameRing(4);
    for (let i = 0; i < 4; i++) r.push(block(i * 128));
    const f = r.pop()!;
    expect(f).toHaveLength(480);
    expect(Array.from(f)).toEqual(Array.from(block(0, 480)));
    expect(r.queued).toBe(32);
    expect(r.pop()).toBeNull();
  });
  it('pop on empty is null', () => {
    expect(new FrameRing(2).pop()).toBeNull();
  });
  it('overflow drops oldest and never throws', () => {
    const r = new FrameRing(1);
    for (let i = 0; i < 4; i++) r.push(block(i * 128)); // 512 samples into 480
    expect(r.queued).toBe(480);
    expect(r.pop()![0]).toBe(32);
    expect(r.push(block(0, 100))).toBe(0); // push returns the samples it dropped
    expect(r.push(block(0, 400))).toBe(20);
    r.pop();
    expect(() => r.push(block(0, 2000))).not.toThrow();
    expect(r.pop()![479]).toBe(1999);
  });
  it('wraps around the buffer', () => {
    const r = new FrameRing(2);
    let n = 0;
    for (let k = 0; k < 30; k++) {
      r.push(block(k * 128));
      for (let f; (f = r.pop()); n += 480) expect(f[0]).toBe(n);
    }
    expect(n).toBe(3840);
  });
});
