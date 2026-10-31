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

// What str0m 0.24 emits for Opus (codec_config.rs enable_opus): no ptime, no bitrate, no DTX.
const str0m = crlf([
  'v=0', 'o=str0m-1 1 2 IN IP4 0.0.0.0', 's=-', 't=0 0', 'a=group:BUNDLE 0',
  'm=audio 9 UDP/TLS/RTP/SAVPF 111',
  'c=IN IP4 0.0.0.0',
  'a=mid:0',
  'a=recvonly',
  'a=rtpmap:111 opus/48000/2',
  'a=fmtp:111 minptime=10;useinbandfec=1',
  'a=rtcp-mux',
]);

// SFU renegotiation offer: own audio, a non-Opus section with its own ptime, and a peer's audio.
const multi = crlf([
  'v=0', 'o=str0m-1 1 3 IN IP4 0.0.0.0', 's=-', 't=0 0', 'a=group:BUNDLE 0 1 2',
  'm=audio 9 UDP/TLS/RTP/SAVPF 111', 'a=mid:0', 'a=recvonly',
  'a=rtpmap:111 opus/48000/2', 'a=fmtp:111 minptime=10;useinbandfec=1',
  'm=audio 9 RTP/AVP 0', 'a=mid:1', 'a=rtpmap:0 PCMU/8000', 'a=ptime:30',
  'm=audio 9 UDP/TLS/RTP/SAVPF 111', 'a=mid:2', 'a=sendonly',
  'a=rtpmap:111 opus/48000/2', 'a=fmtp:111 minptime=10;useinbandfec=1',
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
  it('tunes a str0m-style answer (what the browser configures its encoder from)', () => {
    const out = tuneOpus(str0m, o);
    check(out, '111');
    expect(out).toContain('minptime=10');
  });
  it('omits DTX on request in a str0m answer', () => {
    const out = tuneOpus(str0m, { ...o, dtx: false });
    expect(out).not.toContain('usedtx=1');
  });
  it('munges only inside the Opus m-sections of a multi-section SFU SDP', () => {
    const out = tuneOpus(multi, o).split('\r\n');
    const sec = (n: number) => {
      const s = out.flatMap((l, i) => (l.startsWith('m=') ? [i] : []));
      return out.slice(s[n], s[n + 1] ?? out.length);
    };
    // session section untouched
    expect(out.slice(0, out.findIndex((l) => l.startsWith('m=')))).toEqual(multi.split('\r\n').slice(0, 5));
    for (const n of [0, 2]) {
      expect(sec(n).filter((l) => l === 'a=ptime:20')).toHaveLength(1);
      expect(sec(n).find((l) => l.startsWith('a=fmtp:111 '))).toContain('maxaveragebitrate=64000');
    }
    // the non-Opus section keeps its own ptime and gets no fmtp
    expect(sec(1)).toEqual(multi.split('\r\n').slice(multi.split('\r\n').indexOf('m=audio 9 RTP/AVP 0'), multi.split('\r\n').indexOf('m=audio 9 UDP/TLS/RTP/SAVPF 111', 10)));
    expect(out.join('\r\n').match(/^a=ptime:/gm)).toHaveLength(3);
  });
  it('returns SDP without Opus unchanged', () => {
    const s = crlf(['v=0', 'm=audio 9 RTP/AVP 0', 'a=rtpmap:0 PCMU/8000']);
    expect(tuneOpus(s, o)).toBe(s);
  });
});
