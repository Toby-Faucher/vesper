/** The share card's figures line. Not a live region: the numbers change every poll. */
export function shareLine(s: { width: number; height: number; fps: number; kbps: number; watchers: number }): string {
  const rest = `${s.watchers} watching`;
  if (!s.width) return `Starting · ${rest}`;
  return `${s.width}x${s.height} · ${Math.round(s.fps)} fps · ${+(s.kbps / 1000).toFixed(1)} Mbps · ${rest}`;
}
