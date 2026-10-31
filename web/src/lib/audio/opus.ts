export function tuneOpus(
  sdp: string,
  o: { bitrateKbps: number; dtx: boolean; fec: boolean },
): string {
  const nl = sdp.includes('\r\n') ? '\r\n' : '\n';
  const lines = sdp.split(/\r?\n/);
  const rtpmap = lines.findIndex((l) => /^a=rtpmap:\d+ opus\/48000/i.test(l));
  if (rtpmap < 0) return sdp;
  const pt = lines[rtpmap].match(/^a=rtpmap:(\d+)/)![1];

  const want: Record<string, string> = {
    maxaveragebitrate: String(Math.round(o.bitrateKbps * 1000)),
    usedtx: o.dtx ? '1' : '0',
    useinbandfec: o.fec ? '1' : '0',
    stereo: '0',
  };
  const fmtp = lines.findIndex((l) => l.startsWith(`a=fmtp:${pt} `));
  const params = new Map<string, string>();
  if (fmtp >= 0) {
    for (const kv of lines[fmtp].slice(`a=fmtp:${pt} `.length).split(';')) {
      const [k, v] = kv.split('=');
      if (k) params.set(k.trim(), v ?? '');
    }
  }
  for (const [k, v] of Object.entries(want)) params.set(k, v);
  const line =
    `a=fmtp:${pt} ` + [...params].map(([k, v]) => (v === '' ? k : `${k}=${v}`)).join(';');
  if (fmtp >= 0) lines[fmtp] = line;
  else lines.splice(rtpmap + 1, 0, line);

  // ptime is media-level: replace the existing one or put it after the fmtp line.
  const ptime = lines.findIndex((l) => l.startsWith('a=ptime:'));
  if (ptime >= 0) lines[ptime] = 'a=ptime:20';
  else lines.splice(lines.indexOf(line) + 1, 0, 'a=ptime:20');
  return lines.join(nl);
}
