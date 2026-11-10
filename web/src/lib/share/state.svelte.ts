// Screen shares in the current call, for the call bar and the share panel. voice.svelte.ts fills and clears it.
import { loadLayout, type Layout, type Prefer } from './prefs';

/** A pointer click on a share, shown until `until` (Date.now() ms). */
export type Marker = { share: string; from: string; x: number; y: number; until: number };
/** My own share. */
export type Mine = { track: MediaStreamTrack; fps: 30 | 60; kbps: number; prefer: Prefer; watchers: number };

// Sets and Maps are replaced, not changed, so readers see the change (as voice.speaking).
class Shares {
  /** Remote video by user_id. Every member who can send video has one, sharing or not. */
  streams = $state.raw(new Map<string, MediaStream>());
  hidden = $state(new Set<string>());
  /** Tiles mounted in an open panel. */
  shown = $state(new Set<string>());
  popped = $state<string | null>(null);
  layout = $state<Layout>(loadLayout());
  focus = $state<string | null>(null);
  panelOpen = $state(true);
  chatOpen = $state(false);
  tabVisible = $state(true);
  markers = $state.raw<Marker[]>([]);
  mine = $state.raw<Mine | null>(null);
}
export const shares = new Shares();

/** The call ended: everything of it goes, without telling the server. My own track is stopped by the caller, and the watch
 * gates are reset by the call code (watch.ts). */
export function clearShares(): void {
  Object.assign(shares, {
    streams: new Map(), hidden: new Set(), shown: new Set(), popped: null, focus: null, panelOpen: true, chatOpen: false, markers: [], mine: null,
  });
}
