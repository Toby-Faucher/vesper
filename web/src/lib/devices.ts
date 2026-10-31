export type DeviceError = 'denied' | 'no-device' | 'in-use' | 'unplugged' | 'ended' | 'muted' | 'unknown';

/** Accepts an Error/DOMException, or a capture "microphone lost: <Name>: ..." reason string. */
export function classifyMediaError(e: unknown): DeviceError {
  const s = typeof e === 'string' ? e : ((e as { name?: unknown } | null)?.name ?? '');
  if (typeof s !== 'string') return 'unknown';
  if (/NotAllowedError|SecurityError/.test(s)) return 'denied';
  if (/NotFoundError|OverconstrainedError/.test(s)) return 'no-device';
  if (/NotReadableError/.test(s)) return 'in-use';
  if (/^microphone lost: ended/.test(s)) return 'ended';
  if (/^microphone lost: muted/.test(s)) return 'muted';
  return 'unknown';
}

export const DEVICE_MESSAGES: Record<DeviceError, string> = {
  denied: 'Microphone access was denied. Allow it in the browser site settings; the call stays connected.',
  'no-device': 'No microphone found. Plug one in or pick another; the call stays connected.',
  'in-use': 'The microphone is in use by another app. Close it or pick another device.',
  unplugged: 'The selected microphone was unplugged. Pick another device; the call stays connected.',
  ended: 'The microphone stopped (unplugged, permission revoked or taken by the system). Pick a device; the call stays connected.',
  muted: 'The microphone has been muted by the system or another app; the call stays connected.',
  unknown: 'The microphone failed. Try another device.',
};

/** Headset profiles (HFP) drop to 8 to 16 kHz mono, which wrecks the denoiser input. */
export const isBluetoothLabel = (label: string) => /bluetooth|hands-free|\bhfp\b|airpods|\bbuds/i.test(label);

/** Peak levels in dBFS over a recent window. */
export function levelHint(peaksDb: number[]): 'clipping' | 'too-quiet' | 'ok' {
  if (!peaksDb.length) return 'ok';
  const max = Math.max(...peaksDb);
  return max >= -1 ? 'clipping' : max < -40 ? 'too-quiet' : 'ok';
}

export const missingDevice = (list: MediaDeviceInfo[], id: string | null) =>
  id !== null && !list.some((d) => d.kind === 'audioinput' && d.deviceId === id);

// Before mic permission the browser reports empty ids/labels; those lists must not make every mic look new.
const named = (l: MediaDeviceInfo[]) => l.filter((d) => d.kind === 'audioinput' && d.deviceId && d.label);
export const newInputs = (prev: MediaDeviceInfo[], next: MediaDeviceInfo[]) => {
  const before = named(prev);
  return before.length ? named(next).filter((d) => !before.some((p) => p.deviceId === d.deviceId)) : [];
};

/** Mute always wins; with push to talk the track is live only while the key is held. */
export const trackLive = (muted: boolean, pttEnabled: boolean, held: boolean) => !muted && (!pttEnabled || held);

export function watchDevices(
  cb: (list: MediaDeviceInfo[]) => void,
  md: MediaDevices = navigator.mediaDevices,
): () => void {
  const on = () => void md.enumerateDevices().then(cb, () => {});
  md.addEventListener('devicechange', on);
  return () => md.removeEventListener('devicechange', on);
}

/** Live bitrate change; the SDP maxaveragebitrate from join stays as the negotiated ceiling hint. */
export async function applyBitrate(sender: RTCRtpSender, kbps: number): Promise<void> {
  const p = sender.getParameters();
  const encodings = p.encodings?.length ? p.encodings : [{}];
  encodings.forEach((e) => (e.maxBitrate = Math.round(kbps * 1000)));
  await sender.setParameters({ ...p, encodings });
}
