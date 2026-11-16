// Where the desktop app downloads from: the newest GitHub release. A release attaches its builds under these fixed
// names (docs/versions.md, "How a release is cut"), so /releases/latest/download/<name> always gets the newest one.
const RELEASES = 'https://github.com/Toby-Faucher/vesper/releases';
export const RELEASES_PAGE = `${RELEASES}/latest`;

export type DesktopOs = 'windows' | 'linux';
const download = (file: string) => ({ file, url: `${RELEASES}/latest/download/${file}` });
export const DOWNLOADS: Record<DesktopOs, { label: string; file: string; url: string }> = {
  windows: { label: 'Windows', ...download('vesper-desktop-windows-x64-setup.exe') },
  linux: { label: 'Linux', ...download('vesper-desktop-linux-x64.tar.gz') },
};
/** The Windows build without the installer: a zip to unpack anywhere. */
export const PORTABLE = download('vesper-desktop-windows-x64.zip');

/** The build that fits this browser's system, or null when there is none (macOS, phones). */
export function desktopOs(userAgent: string): DesktopOs | null {
  if (/Android|iPhone|iPad|CrOS/i.test(userAgent)) return null;
  if (/Windows/i.test(userAgent)) return 'windows';
  if (/Linux|X11/i.test(userAgent)) return 'linux';
  return null;
}

/** Opens an invite in the desktop app (`desktop/src/single.rs` reads it). It carries this server's address, which the
 * desktop app cannot know. Like the invite page's own address, it is never sent to the server. */
export function inviteLink(code: string, origin: string): string {
  return `vesper://invite/${code}?server=${encodeURIComponent(origin)}`;
}
