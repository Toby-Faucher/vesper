import { describe, expect, it } from 'vitest';
import { tuneOpus } from './opus';

const o = { bitrateKbps: 64, dtx: true, fec: true };
const crlf = (l: string[]) => l.join('\r\n') + '\r\n';

const chrome = crlf([
  'v=0', 'o=- 1 2 IN IP4 127.0.0.1', 's=-', 't=0 0',
  'm=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0',
  'a=rtpmap:111 opus/48000/2',
  'a=rtcp-fb:111 transport-cc',
  'a=fmtp:111 minptime=10;useinbandfec=0',
  'a=rtpmap:63 red/48000/2',
  'a=fmtp:63 111/111',
  'a=rtpmap:9 G722/8000',
  'a=rtpmap:0 PCMU/8000',
]);

const firefox = crlf([
  'v=0', 's=-', 't=0 0',
  'm=audio 9 UDP/TLS/RTP/SAVPF 109 9 0',
  'a=rtpmap:109 opus/48000/2',
  'a=rtpmap:9 G722/8000/1',
  'a=fmtp:109 maxplaybackrate=48000;stereo=1;useinbandfec=1',
  'a=ptime:20',
  'a=maxptime:20',
]);

function check(out: string, pt: string) {
  const fmtp = out.split(/\r?\n/).find((l) => l.startsWith(`a=fmtp:${pt} `))!;
  for (const p of ['maxaveragebitrate=64000', 'usedtx=1', 'useinbandfec=1', 'stereo=0']) {
    expect(fmtp.split(' ')[1].split(';')).toContain(p);
  }
  expect(fmtp).not.toMatch(/useinbandfec=0|stereo=1/);
  expect(out.match(/^a=ptime:20$/gm)).toHaveLength(1);
}

describe('tuneOpus', () => {
  it('tunes a Chrome SDP', () => {
    const out = tuneOpus(chrome, o);
    check(out, '111');
    expect(out).toContain('minptime=10');
    expect(out).toContain('a=fmtp:63 111/111');
    expect(out).toContain('a=rtpmap:9 G722/8000');
    expect(out).toContain('\r\n');
  });
  it('tunes a Firefox SDP', () => {
    const out = tuneOpus(firefox, o);
    check(out, '109');
    expect(out).toContain('maxplaybackrate=48000');
  });
  it('adds an fmtp line when none exists', () => {
    const out = tuneOpus(chrome.replace(/a=fmtp:111 .*\r\n/, ''), o);
    check(out, '111');
  });
  it('honours dtx/fec off and other bitrates', () => {
    const out = tuneOpus(chrome, { bitrateKbps: 32, dtx: false, fec: false });
    expect(out).toContain('maxaveragebitrate=32000');
    expect(out).toContain('usedtx=0');
    expect(out).toContain('useinbandfec=0');
  });
  it('returns SDP without Opus unchanged', () => {
    const s = crlf(['v=0', 'm=audio 9 RTP/AVP 0', 'a=rtpmap:0 PCMU/8000']);
    expect(tuneOpus(s, o)).toBe(s);
  });
});
