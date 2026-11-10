import { afterEach, describe, expect, it, vi } from 'vitest';
import { joinRoom } from './join';
import type { Signaling } from './signaling';
import type { ServerMsg } from './gen/ServerMsg';

const tick = () => new Promise((r) => setTimeout(r, 0));

class FakePC {
  static last: FakePC;
  log: string[] = [];
  active = 0;
  maxActive = 0;
  failFirst = false;
  closed = false;
  ontrack: unknown;
  remotes: { type: string; sdp: string }[] = [];
  params: { encodings: RTCRtpEncodingParameters[] } = { encodings: [{}] };
  sender = {
    track: { kind: 'audio' },
    getParameters: () => this.params,
    setParameters: async (p: { encodings: RTCRtpEncodingParameters[] }) => { this.params = p; },
  };
  video = { track: null };
  added: unknown[][] = []; // addTrack and addTransceiver calls, in order
  offered: unknown[][] = []; // `added` when the offer was made
  constructor() { FakePC.last = this; }
  getSenders() { return [this.sender, this.video]; }
  addTrack(t: { kind: string }) { this.added.push(['track', t.kind]); }
  addTransceiver(kind: string, init: unknown) { this.added.push(['transceiver', kind, init]); return { sender: this.video }; }
  close() { this.closed = true; }
  async createOffer() { this.offered = [...this.added]; return { type: 'offer', sdp: 'v=0' }; }
  async createAnswer() { return { type: 'answer', sdp: 'ans' }; }
  async setLocalDescription(d: { type: string; sdp: string }) { this.log.push(`local:${d.type}`); }
  async setRemoteDescription(d: { type: string; sdp: string }) {
    this.remotes.push(d);
    if (d.type !== 'offer') return;
    this.maxActive = Math.max(this.maxActive, ++this.active);
    this.log.push(`remote:${d.sdp}`);
    await tick();
    this.active--;
    if (this.failFirst && d.sdp === 'o1') throw new Error('boom');
  }
}

function setup() {
  vi.stubGlobal('RTCPeerConnection', FakePC);
  const sent: any[] = [];
  let msg: (m: ServerMsg) => void = () => {};
  let close: () => void = () => {};
  const sig = {
    send: (m: unknown) => sent.push(m),
    onMessage: (cb: (m: ServerMsg) => void) => ((msg = cb), () => (msg = () => {})),
    onClose: (cb: () => void) => ((close = cb), () => (close = () => {})),
  } as unknown as Signaling;
  const stop = vi.fn();
  const stream = { getAudioTracks: () => [], getTracks: () => [{ stop }] } as unknown as MediaStream;
  const onError = vi.fn();
  return { sent, sig, stream, onError, stop, emit: (m: ServerMsg) => msg(m), close: () => close() };
}
const joined: ServerMsg = { type: 'joined', user_id: 'u', answer: 'a', peers: [] };
afterEach(() => vi.unstubAllGlobals());

const str0mSdp = (dir: string) =>
  ['v=0', 's=-', 't=0 0', 'm=audio 9 UDP/TLS/RTP/SAVPF 111', 'a=mid:0', `a=${dir}`,
    'a=rtpmap:111 opus/48000/2', 'a=fmtp:111 minptime=10;useinbandfec=1', ''].join('\r\n');

describe('joinRoom', () => {
  for (const dtx of [true, false]) {
    it(`tunes the server answer and sets maxBitrate at join (dtx ${dtx})`, async () => {
      const t = setup();
      const p = joinRoom(t.sig, { roomId: 1, stream: t.stream, bitrateKbps: 96, dtx });
      await tick();
      t.emit({ ...joined, answer: str0mSdp('recvonly') } as ServerMsg);
      await p;
      const ans = FakePC.last.remotes.find((d) => d.type === 'answer')!.sdp;
      const fmtp = ans.split('\r\n').find((l) => l.startsWith('a=fmtp:111 '))!.slice(11).split(';');
      expect(fmtp).toEqual(expect.arrayContaining(['maxaveragebitrate=96000', 'useinbandfec=1', 'stereo=0']));
      expect(fmtp).toContain(dtx ? 'usedtx=1' : 'usedtx=0');
      expect(ans.match(/^a=ptime:20$/gm)).toHaveLength(1);
      expect(FakePC.last.params.encodings[0]).toMatchObject({ maxBitrate: 96000, priority: 'high', networkPriority: 'high' });
    });
  }
  it('a voice priority the browser refuses does not fail the join', async () => {
    const t = setup();
    const p = joinRoom(t.sig, { roomId: 1, stream: t.stream, bitrateKbps: 64, onError: t.onError });
    await tick();
    const set = FakePC.last.sender.setParameters;
    FakePC.last.sender.setParameters = async (x) => {
      if (x.encodings[0].priority) throw new Error('no priority');
      return set(x);
    };
    t.emit(joined);
    await expect(p).resolves.toBeTruthy();
    expect(FakePC.last.params.encodings[0].maxBitrate).toBe(64000);
    expect(t.onError).not.toHaveBeenCalled();
  });
  it('offers the audio track and one send-only video transceiver, and returns its sender', async () => {
    const t = setup();
    const mic = { kind: 'audio' };
    const stream = { ...t.stream, getAudioTracks: () => [mic] } as unknown as MediaStream;
    const p = joinRoom(t.sig, { roomId: 1, stream, bitrateKbps: 64 });
    await tick();
    expect(FakePC.last.offered).toEqual([['track', 'audio'], ['transceiver', 'video', { direction: 'sendonly' }]]);
    t.emit(joined);
    expect((await p).video).toBe(FakePC.last.video);
  });
  it('tunes SFU renegotiation offers before applying them', async () => {
    const t = setup();
    const p = joinRoom(t.sig, { roomId: 1, stream: t.stream, bitrateKbps: 64 });
    await tick();
    t.emit(joined);
    await p;
    t.emit({ type: 'offer', sdp: str0mSdp('sendrecv') });
    await tick(); await tick();
    const off = FakePC.last.remotes.find((d) => d.type === 'offer')!.sdp;
    expect(off).toContain('maxaveragebitrate=64000');
    expect(off).toContain('usedtx=1');
  });

  it('serializes offers; failure does not block the next', async () => {
    const t = setup();
    const p = joinRoom(t.sig, { roomId: 1, stream: t.stream, bitrateKbps: 64, onError: t.onError });
    await tick();
    FakePC.last.failFirst = true;
    t.emit(joined);
    await p;
    t.emit({ type: 'offer', sdp: 'o1' });
    t.emit({ type: 'offer', sdp: 'o2' });
    await tick(); await tick(); await tick();
    expect(FakePC.last.maxActive).toBe(1);
    expect(t.sent.filter((m) => m.type === 'answer')).toEqual([{ type: 'answer', sdp: 'ans' }]);
    expect(FakePC.last.log.filter((l) => l.startsWith('remote:'))).toEqual(['remote:o1', 'remote:o2']);
    expect(t.onError).toHaveBeenCalledOnce();
  });
  it('answers two good offers in order', async () => {
    const t = setup();
    const p = joinRoom(t.sig, { roomId: 1, stream: t.stream, bitrateKbps: 64 });
    await tick();
    t.emit(joined);
    await p;
    t.emit({ type: 'offer', sdp: 'o1' });
    t.emit({ type: 'offer', sdp: 'o2' });
    await tick(); await tick(); await tick();
    expect(FakePC.last.maxActive).toBe(1);
    expect(t.sent.filter((m) => m.type === 'answer')).toHaveLength(2);
  });
  it('rejoin_plays_the_new_track (#45): a new track for a known id is played, alone, and the old one ending does not drop it', async () => {
    vi.stubGlobal('MediaStream', class { id = crypto.randomUUID(); constructor(public tracks: unknown[]) {} });
    const track = () => {
      const t = { kind: 'audio', readyState: 'live', ends: [] as (() => void)[], addEventListener: (_: string, f: () => void) => t.ends.push(f) };
      return t;
    };
    const end = (t: ReturnType<typeof track>) => ((t.readyState = 'ended'), t.ends.forEach((f) => f()));
    const t = setup();
    const added: [string, unknown[]][] = [];
    const ended: string[] = [];
    const p = joinRoom(t.sig, { roomId: 1, stream: t.stream, bitrateKbps: 64,
      onRemoteStream: (id, s, kind) => kind === 'audio' && added.push([id, (s as unknown as { tracks: unknown[] }).tracks]),
      onRemoteStreamEnd: (id, kind) => kind === 'audio' && ended.push(id) });
    await tick();
    t.emit(joined);
    await p;
    const ontrack = FakePC.last.ontrack as (e: unknown) => void;
    // Firefox: one MediaStream per MSID id, and a stopped m-line's ended track stays in it with no removetrack.
    const bob = { id: 'b'.repeat(32), tracks: [] as unknown[], getTracks: () => bob.tracks };
    const old = track();
    bob.tracks.push(old);
    ontrack({ track: old, streams: [bob] });
    end(old); // bob left: the SFU stopped his m-line
    const fresh = track();
    bob.tracks.push(fresh);
    ontrack({ track: fresh, streams: [bob] }); // bob is back
    end(old); // a second, late end of the old track
    expect(added).toEqual([[bob.id, [old]], [bob.id, [fresh]]]);
    expect(ended).toEqual([bob.id]);
    end(fresh);
    expect(ended).toEqual([bob.id, bob.id]);
  });
  it('a video track and an audio track for one user are both live: neither ends the other', async () => {
    vi.stubGlobal('MediaStream', class { id = crypto.randomUUID(); constructor(public tracks: unknown[]) {} });
    const track = (kind: string) => {
      const t = { kind, ends: [] as (() => void)[], addEventListener: (_: string, f: () => void) => t.ends.push(f) };
      return t;
    };
    const t = setup();
    const added: [string, string, unknown[]][] = [];
    const ended: [string, string][] = [];
    const p = joinRoom(t.sig, { roomId: 1, stream: t.stream, bitrateKbps: 64,
      onRemoteStream: (id, s, kind) => added.push([id, kind, (s as unknown as { tracks: unknown[] }).tracks]),
      onRemoteStreamEnd: (id, kind) => ended.push([id, kind]) });
    await tick();
    t.emit(joined);
    await p;
    const ontrack = FakePC.last.ontrack as (e: unknown) => void;
    const u1 = { id: 'u1' };
    const [screen, voice] = [track('video'), track('audio')];
    ontrack({ track: screen, streams: [u1] });
    ontrack({ track: voice, streams: [u1] });
    expect(added).toEqual([['u1', 'video', [screen]], ['u1', 'audio', [voice]]]);
    expect(ended).toEqual([]);
    screen.ends.forEach((f) => f());
    expect(ended).toEqual([['u1', 'video']]);
    voice.ends.forEach((f) => f());
    expect(ended).toEqual([['u1', 'video'], ['u1', 'audio']]);
  });
  it('reports post-join errors and cleans up on close', async () => {
    const t = setup();
    const p = joinRoom(t.sig, { roomId: 1, stream: t.stream, bitrateKbps: 64, onError: t.onError });
    await tick();
    t.emit(joined);
    await p;
    t.emit({ type: 'error', code: 'bad_message', message: 'x' });
    expect(t.onError).toHaveBeenCalledOnce();
    expect(t.onError.mock.calls[0][0].message).not.toMatch(/bad_message/);
    t.close();
    expect(FakePC.last.closed).toBe(true);
    expect(t.stop).toHaveBeenCalled();
  });
});
