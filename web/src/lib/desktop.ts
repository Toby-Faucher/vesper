// Where the desktop app downloads from: the newest GitHub release. A release attaches its builds under these fixed
// names (docs/versions.md, "How a release is cut"), so /releases/latest/download/<name> always gets the newest one.
const RELEASES = 'https://github.com/Toby-Faucher/vesper/releases';
export const RELEASES_PAGE = `${RELEASES}/latest`;

export type DesktopOs = 'windows' | 'linux';
export const DOWNLOADS: Record<DesktopOs, { label: string; file: string; url: string }> = {
  windows: { label: 'Windows', file: 'vesper-desktop-windows-x64.zip', url: `${RELEASES}/latest/download/vesper-desktop-windows-x64.zip` },
  linux: { label: 'Linux', file: 'vesper-desktop-linux-x64.tar.gz', url: `${RELEASES}/latest/download/vesper-desktop-linux-x64.tar.gz` },
};

/** The build that fits this browser's system, or null when there is none (macOS, phones). */
export function desktopOs(userAgent: string): DesktopOs | null {
  if (/Android|iPhone|iPad|CrOS/i.test(userAgent)) return null;
  if (/Windows/i.test(userAgent)) return 'windows';
  if (/Linux|X11/i.test(userAgent)) return 'linux';
  return null;
}
