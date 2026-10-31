export type DenoiseMode = 'deepfilter' | 'browser' | 'off';

/**
 * Trips to 'browser' once `maxLate` of the last `windowFrames` frames took longer than `budgetMs`.
 * The first `warmupFrames` reports are ignored.
 */
export class ModeController {
  private o: { budgetMs: number; windowFrames: number; maxLate: number; warmupFrames?: number };
  private seen = 0;
  private late: boolean[] = [];
  private count = 0;
  private tripped = false;

  constructor(o: { budgetMs: number; windowFrames: number; maxLate: number; warmupFrames?: number }) {
    this.o = o;
  }

  report(frameMs: number): DenoiseMode {
    if (this.seen < (this.o.warmupFrames ?? 0)) this.seen++;
    else if (!this.tripped) {
      const l = frameMs > this.o.budgetMs;
      this.late.push(l);
      if (l) this.count++;
      if (this.late.length > this.o.windowFrames && this.late.shift()) this.count--;
      this.tripped = this.count >= this.o.maxLate;
    }
    return this.tripped ? 'browser' : 'deepfilter';
  }

  reset(): void {
    this.late = [];
    this.count = 0;
    this.seen = 0;
    this.tripped = false;
  }
}
