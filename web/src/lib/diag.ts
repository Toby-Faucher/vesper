import { Signaling, joinRoom } from './signaling';

export type Diag = {
  rttMs: number | null;
  jitterMs: number | null;
  lossDownPct: number | null;
  lossUpPct: number | null;
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

export function summarize(prev: RTCStatsReport | null, cur: RTCStatsReport, dtSec: number, iceFallback = 'unknown'): Diag {
  const c = all(cur);
  const p = all(prev);
  const byId = (id: unknown) => c.find((x) => x.id === id);
  const audios = (s: Stat[], type: string) => s.filter((x) => x.type === type && (x.kind ?? x.mediaType) === 'audio');
  const transport = c.find((x) => x.type === 'transport');
  const pair =
    byId(transport?.selectedCandidatePairId) ?? c.find((x) => x.type === 'candidate-pair' && x.nominated && x.state === 'succeeded');
  const type = byId(pair?.localCandidateId)?.candidateType;
  const out = audios(c, 'outbound-rtp')[0];
  const pOut = p.find((x) => x.id === out?.id);
  const inns = audios(c, 'inbound-rtp');
  const remote = audios(c, 'remote-inbound-rtp');

  // Incoming loss: sum deltas over streams present in both reports; a counter reset contributes 0.
  let dLost = 0;
  let dRecv = 0;
  for (const i of inns) {
    const o = p.find((x) => x.id === i.id);
    const [l1, r1, l0, r0] = [num(i.packetsLost), num(i.packetsReceived), num(o?.packetsLost), num(o?.packetsReceived)];
    if (l1 === null || r1 === null || l0 === null || r0 === null || l1 < l0 || r1 < r0) continue;
    dLost += l1 - l0;
    dRecv += r1 - r0;
  }
  const lossDownPct = dLost + dRecv > 0 ? (dLost / (dLost + dRecv)) * 100 : null;

  const fr = remote.map((r) => num(r.fractionLost)).filter((v): v is number => v !== null);
  const lossUpPct = fr.length ? (fr.reduce((a, b) => a + b, 0) / fr.length) * 100 : null;

  const [b1, b0] = [num(out?.bytesSent), num(pOut?.bytesSent)];
  const sendKbps = b1 !== null && b0 !== null && b1 >= b0 && dtSec > 0 ? ((b1 - b0) * 8) / dtSec / 1000 : null;

  const rtt = num(pair?.currentRoundTripTime) ?? num(remote.map((r) => num(r.roundTripTime)).find((v) => v !== null));
  const jits = inns.map((i) => num(i.jitter)).filter((v): v is number => v !== null);
  const mime = byId((out ?? inns[0])?.codecId)?.mimeType;
  return {
    rttMs: rtt === null ? null : rtt * 1000,
    jitterMs: jits.length ? Math.max(...jits) * 1000 : null,
    lossDownPct,
    lossUpPct,
    sendKbps,
    codec: typeof mime === 'string' ? mime.split('/').pop()!.toLowerCase() : null,
    iceState: typeof transport?.iceState === 'string' ? transport.iceState : iceFallback,
    selectedPair: type === 'host' || type === 'srflx' || type === 'relay' ? type : null,
  };
}

export type CheckDeps = {
  newSignaling: (url: string) => Pick<Signaling, 'close' | 'onClose'>;
  newAudioContext: () => AudioContext;
  joinRoom: (sig: any, opts: Parameters<typeof joinRoom>[1]) => Promise<{ pc: RTCPeerConnection }>;
};
const realDeps: CheckDeps = {
  newSignaling: (u) => new Signaling(u),
  newAudioContext: () => new AudioContext(),
  joinRoom,
};
const JOIN_TIMEOUT_MS = 8000;
const ICE_TIMEOUT_MS = 5000; // starts only after joinRoom resolves

export async function connectivityCheck(
  signalingUrl: string,
  key: string,
  deps: CheckDeps = realDeps,
): Promise<{ ok: boolean; reason: string; ms: number }> {
  const t0 = Date.now();
  const done = (ok: boolean, reason: string) => ({ ok, reason, ms: Date.now() - t0 });
  const timers: ReturnType<typeof setTimeout>[] = [];
  const after = <T>(ms: number, v: T) => new Promise<T>((res) => timers.push(setTimeout(() => res(v), ms)));
  let ctx: AudioContext | undefined;
  let sig: Pick<Signaling, 'close' | 'onClose'> | undefined;
  let pc: RTCPeerConnection | undefined;
  let stream: MediaStream | undefined;
  let joining = true;
  try {
    ctx = deps.newAudioContext();
    stream = ctx.createMediaStreamDestination().stream; // silent: no microphone permission needed
    sig = deps.newSignaling(signalingUrl);
    const wsClosed = new Promise<'closed'>((res) => sig!.onClose(() => res('closed')));
    const joined = await Promise.race([
      deps.joinRoom(sig, { room: `check-${Math.random().toString(36).slice(2)}`, name: 'check', key, stream, bitrateKbps: 32 }),
      after(JOIN_TIMEOUT_MS, 'timeout' as const),
    ]);
    if (joined === 'timeout')
      return done(false, 'Signaling failed: the WebSocket did not connect or the server did not answer within 8 s (check the server address).');
    joining = false;
    pc = joined.pc;
    const p = pc;
    const up = () => p.iceConnectionState === 'connected' || p.iceConnectionState === 'completed';
    const ice = new Promise<'up'>((res) => {
      if (up()) res('up');
      p.addEventListener('iceconnectionstatechange', () => up() && res('up'));
    });
    const r = await Promise.race([ice, wsClosed, after(ICE_TIMEOUT_MS, 'timeout' as const)]);
    if (r === 'up') return done(true, 'Media path works (ICE connected).');
    if (r === 'closed') return done(false, 'The signaling connection closed while the media path was being set up (server restarted or dropped the connection); try again.');
    return done(false, 'Signaling worked but media did not connect within 5 s: UDP is probably blocked, or the forwarded UDP port is closed.');
  } catch (e) {
    const m = e instanceof Error ? e.message : String(e);
    return done(false, joining ? `Signaling failed: ${m} (bad key, or the WebSocket could not connect).` : `Check failed after joining: ${m}`);
  } finally {
    timers.forEach(clearTimeout);
    pc?.close();
    stream?.getTracks().forEach((t) => t.stop());
    sig?.close();
    void ctx?.close().catch(() => {});
  }
}
