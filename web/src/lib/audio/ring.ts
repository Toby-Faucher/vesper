/** DeepFilterNet hop: 10 ms at 48 kHz. */
export const HOP = 480;

/** Collects 128-sample worklet blocks and hands out HOP-sample frames; overflow drops the oldest samples. */
export class FrameRing {
  private buf: Float32Array;
  private r = 0;
  private n = 0;

  constructor(capacityFrames: number) {
    this.buf = new Float32Array(capacityFrames * HOP);
  }

  push(block: Float32Array): void {
    const cap = this.buf.length;
    const b = block.length > cap ? block.subarray(block.length - cap) : block;
    const over = this.n + b.length - cap;
    if (over > 0) {
      this.r = (this.r + over) % cap;
      this.n -= over;
    }
    const w = (this.r + this.n) % cap;
    const first = Math.min(b.length, cap - w);
    this.buf.set(b.subarray(0, first), w);
    this.buf.set(b.subarray(first), 0);
    this.n += b.length;
  }

  /** A fresh array each time, so the caller may transfer its buffer. */
  pop(): Float32Array | null {
    if (this.n < HOP) return null;
    const cap = this.buf.length;
    const f = new Float32Array(HOP);
    const first = Math.min(HOP, cap - this.r);
    f.set(this.buf.subarray(this.r, this.r + first));
    f.set(this.buf.subarray(0, HOP - first), first);
    this.r = (this.r + HOP) % cap;
    this.n -= HOP;
    return f;
  }

  get queued(): number {
    return this.n;
  }
}
