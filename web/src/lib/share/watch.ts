export const WATCH_GRACE_MS = 10_000;

/** Watching: tile in the panel with the tab visible, or popped out. */
export const wantsWatch = (s: { inPanel: boolean; tabVisible: boolean; poppedOut: boolean }): boolean =>
  s.poppedOut || (s.inPanel && s.tabVisible);

/** One per sharer. Sends watch on at once and watch off only after the grace, so a quick alt-tab costs nothing. */
export class WatchGate {
  private on = false;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(private send: (on: boolean) => void) {}

  set(want: boolean): void {
    if (want) {
      this.clear();
      if (!this.on) { this.on = true; this.send(true); }
    } else if (this.on && this.timer === null) {
      this.timer = setTimeout(() => {
        this.timer = null;
        this.on = false;
        this.send(false);
      }, WATCH_GRACE_MS);
    }
  }

  /** Cancels a pending off and sends nothing. */
  dispose(): void { this.clear(); }

  private clear(): void {
    if (this.timer !== null) { clearTimeout(this.timer); this.timer = null; }
  }
}
