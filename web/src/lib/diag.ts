import { Signaling, joinRoom } from './signaling';

export type Diag = {
  rttMs: number | null;
  jitterMs: number | null;
  lossPct: number | null;
  sendKbps: number | null;
  codec: string | null;
  iceState: string;
  selectedPair: 'host' | 'srflx' | 'relay' | null;
};

type Stat = Record<string, any>;
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const all = (r: RTCStatsReport | null): Stat[] => {
  const out: Stat[] = [];
  r?.forEach((s) => out.push(s));
  return out;
};
const audio = (s: Stat[], type: string) => s.find((x) => x.type === type && (x.kind ?? x.mediaType) === 'audio');

export function summarize(prev: RTCStatsReport | null, cur: RTCStatsReport, dtSec: number): Diag {
  const c = all(cur);
  const p = all(prev);
  const byId = (id: unknown) => c.find((x) => x.id === id);
  const transport = c.find((x) => x.type === 'transport');
  const pair =
    byId(transport?.selectedCandidatePairId) ?? c.find((x) => x.type === 'candidate-pair' && x.nominated && x.state === 'succeeded');
  const local = byId(pair?.localCandidateId);
  const type = local?.candidateType;
  const out = audio(c, 'outbound-rtp');
  const inn = audio(c, 'inbound-rtp');
  const pOut = audio(p, 'outbound-rtp');
  const pInn = audio(p, 'inbound-rtp');

  let lossPct: number | null = null;
  const [l1, r1, l0, r0] = [num(inn?.packetsLost), num(inn?.packetsReceived), num(pInn?.packetsLost), num(pInn?.packetsReceived)];
  if (l1 !== null && r1 !== null && l0 !== null && r0 !== null) {
    const total = l1 - l0 + (r1 - r0);
    if (total > 0) lossPct = (Math.max(0, l1 - l0) / total) * 100;
  }
  if (lossPct === null) {
    const f = num(audio(c, 'remote-inbound-rtp')?.fractionLost);
    if (f !== null) lossPct = f * 100;
  }

  const [b1, b0] = [num(out?.bytesSent), num(pOut?.bytesSent)];
  const sendKbps = b1 !== null && b0 !== null && dtSec > 0 ? (Math.max(0, b1 - b0) * 8) / dtSec / 1000 : null;

  const rtt = num(pair?.currentRoundTripTime);
  const jit = num(inn?.jitter);
  const mime = byId((out ?? inn)?.codecId)?.mimeType;
  return {
    rttMs: rtt === null ? null : rtt * 1000,
    jitterMs: jit === null ? null : jit * 1000,
    lossPct,
    sendKbps,
    codec: typeof mime === 'string' ? mime.split('/').pop()!.toLowerCase() : null,
    iceState: typeof transport?.iceState === 'string' ? transport.iceState : 'unknown',
    selectedPair: type === 'host' || type === 'srflx' || type === 'relay' ? type : null,
  };
}

export type CheckDeps = {
  newSignaling: (url: string) => Pick<Signaling, 'close'>;
  newAudioContext: () => AudioContext;
  joinRoom: (sig: any, opts: Parameters<typeof joinRoom>[1]) => Promise<{ pc: RTCPeerConnection }>;
};
const realDeps: CheckDeps = {
  newSignaling: (u) => new Signaling(u),
  newAudioContext: () => new AudioContext(),
  joinRoom,
};
const TIMEOUT_MS = 5000;

export async function connectivityCheck(
  signalingUrl: string,
  key: string,
  deps: CheckDeps = realDeps,
): Promise<{ ok: boolean; reason: string; ms: number }> {
  const t0 = Date.now();
  const done = (ok: boolean, reason: string) => ({ ok, reason, ms: Date.now() - t0 });
  let ctx: AudioContext | undefined;
  let sig: Pick<Signaling, 'close'> | undefined;
  let pc: RTCPeerConnection | undefined;
  let stream: MediaStream | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<'timeout'>((res) => (timer = setTimeout(() => res('timeout'), TIMEOUT_MS)));
  try {
    ctx = deps.newAudioContext();
    stream = ctx.createMediaStreamDestination().stream; // silent: no microphone permission needed
    sig = deps.newSignaling(signalingUrl);
    const joined = await Promise.race([
      deps.joinRoom(sig, { room: `check-${Math.random().toString(36).slice(2)}`, name: 'check', key, stream, bitrateKbps: 32 }),
      timeout,
    ]);
    if (joined === 'timeout')
      return done(false, 'Signaling failed: the WebSocket did not connect or the server did not answer within 5 s (check the server address).');
    pc = joined.pc;
    const p = pc;
    const up = () => p.iceConnectionState === 'connected' || p.iceConnectionState === 'completed';
    const ice = new Promise<'up'>((res) => {
      if (up()) res('up');
      p.addEventListener('iceconnectionstatechange', () => up() && res('up'));
    });
    return (await Promise.race([ice, timeout])) === 'up'
      ? done(true, 'Media path works (ICE connected).')
      : done(false, 'Signaling worked but media did not connect within 5 s: UDP is probably blocked, or the forwarded UDP port is closed.');
  } catch (e) {
    return done(false, `Signaling failed: ${e instanceof Error ? e.message : String(e)} (bad key, or the WebSocket could not connect).`);
  } finally {
    clearTimeout(timer);
    pc?.close();
    stream?.getTracks().forEach((t) => t.stop());
    sig?.close();
    void ctx?.close().catch(() => {});
  }
}
