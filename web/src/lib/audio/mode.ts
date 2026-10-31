export type DenoiseMode = 'deepfilter' | 'browser' | 'off';

/** Trips to 'browser' once `maxLate` of the last `windowFrames` frames took longer than `budgetMs`. */
export class ModeController {
  private o: { budgetMs: number; windowFrames: number; maxLate: number };
  private late: boolean[] = [];
  private count = 0;
  private tripped = false;

  constructor(o: { budgetMs: number; windowFrames: number; maxLate: number }) {
    this.o = o;
  }

  report(frameMs: number): DenoiseMode {
    if (!this.tripped) {
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
    this.tripped = false;
  }
}
