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

  // Index loops, not subarray/set: no garbage on the audio thread.
  push(block: Float32Array): void {
    const cap = this.buf.length;
    const skip = Math.max(0, block.length - cap);
    const len = block.length - skip;
    const over = this.n + len - cap;
    if (over > 0) {
      this.r = (this.r + over) % cap;
      this.n -= over;
    }
    let w = (this.r + this.n) % cap;
    for (let i = skip; i < block.length; i++) {
      this.buf[w] = block[i];
      if (++w === cap) w = 0;
    }
    this.n += len;
  }

  /** A fresh array each time, so the caller may transfer its buffer. */
  pop(): Float32Array | null {
    if (this.n < HOP) return null;
    const cap = this.buf.length;
    const f = new Float32Array(HOP);
    for (let i = 0; i < HOP; i++) {
      f[i] = this.buf[this.r];
      if (++this.r === cap) this.r = 0;
    }
    this.n -= HOP;
    return f;
  }

  get queued(): number {
    return this.n;
  }
}
