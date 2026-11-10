import { shares } from './state.svelte';

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

const gates = new Map<string, WatchGate>();

/** Sends watch on or off for each sharer in the call. A share that ended keeps its gate, so the off still goes out. */
export function syncWatches(sharing: string[], send: (userId: string, on: boolean) => void): void {
  for (const id of new Set([...sharing, ...gates.keys()])) {
    let g = gates.get(id);
    if (!g) gates.set(id, (g = new WatchGate((on) => send(id, on))));
    g.set(sharing.includes(id) && wantsWatch({ inPanel: shares.shown.has(id), tabVisible: shares.tabVisible, poppedOut: shares.popped === id }));
  }
}

/** A new call: the server forgot this client's watches, so the next syncWatches sends them again. Sends nothing. */
export function resetWatches(): void {
  gates.forEach((g) => g.dispose());
  gates.clear();
}
