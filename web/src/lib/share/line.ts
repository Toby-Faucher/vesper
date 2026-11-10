type Figures = { width: number; height: number; fps: number; kbps: number };

/** Size, frame rate and bitrate, as the share card and a tile's hover show them. */
export const figures = (s: Figures): string => `${s.width}x${s.height} · ${Math.round(s.fps)} fps · ${+(s.kbps / 1000).toFixed(1)} Mbps`;

/** The share card's figures line. Not a live region: the numbers change every poll. */
export function shareLine(s: Figures & { watchers: number }): string {
  const rest = `${s.watchers} watching`;
  return s.width ? `${figures(s)} · ${rest}` : `Starting · ${rest}`;
}
