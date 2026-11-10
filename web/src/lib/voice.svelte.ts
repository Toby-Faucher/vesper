// The voice call, moved from Room.svelte so it outlives the screen that shows it.
import { tick, untrack } from 'svelte';
import { callEvent, idleCall } from './call';
import { onLogout, session } from './session.svelte';
import type { Playback } from './playback';
import { restoreVolumes, saveVolumes } from './volume';
import type { Capture } from './audio/capture';
import { captureContext } from './audio/context';
import { loadSettings, saveSettings, type AudioSettings } from './audio/settings';
import { DEVICE_MESSAGES, MIC_FALLBACK, SPEAKER_FALLBACK, applyBitrate, classifyMediaError, deviceGone, micStartError, missingDevice, newInputs, trackLive, watchDevices } from './devices';
import { pttKeyEvent } from './audio/ptt';
import { isTyping } from './ui/keys';
import { MIC_QUIET, micSilence, type MicWarning } from './audio/mic-silence';
import type { DenoiseMode } from './audio/mode';
import type { Diag } from './diag';
import type { Quality } from './ui/quality';
import type { Link } from './ui/link';
import { callCue, type CallSnap } from './ui/cues';
import { chime } from './ui/chime';
import { loadLevelling, loadPrefs, saveLevelling } from './ui/prefs';
import { lazy } from './ui/lazy';
import { api } from './api';
import type { CallTally } from './call-summary';
import type { ServerMsg } from './gen/ServerMsg';
import type { ShareSettings } from './share/prefs';
import { clearShares, resetWatches, shares } from './share/state.svelte';

// Accessing localStorage itself throws when storage is blocked.
const storage = (() => { try { return localStorage; } catch { return null; } })();

class Voice {
  joinedRoom = $state<number | null>(null);
  joining = $state(false);
  forceMuted = $state(false);
  joined = $state(false);
  error = $state('');
  userId = $state('');
  muted = $state(false);
  deafened = $state(false);
  volumes = $state<Record<string, number>>({});
  speaking = $state<Set<string>>(new Set());
  mic = $state.raw<MediaStream>();
  capture = $state.raw<Capture>();
  settings = $state<AudioSettings>(loadSettings(storage));
  joinedBitrate = $state(64); // set at join
  pc = $state.raw<RTCPeerConnection>();
  deviceError = $state('');
  /** A saved mic or speaker was gone, so the default one is used. */
  notice = $state('');
  offered = $state<MediaDeviceInfo | null>(null);
  pttHeld = $state(false);
  mode = $state<DenoiseMode>('deepfilter');
  modeReason = $state('');
  micWarning = $state<MicWarning>(null);
  quality = $state<Quality>('unknown');
  /** The last stats summary behind `quality`, for qualityText. */
  diag = $state.raw<Diag | null>(null);
  /** Connection quality per person in the call (you too), keyed on user_id. Empty when not in a call. */
  links = $state.raw<Record<string, Link>>({});
  /** True while Retry microphone runs. */
  retrying = $state(false);
  /** "Even out voices" (Settings > Voice), saved in this browser. */
  levelling = $state(loadLevelling());
}
export const voice = new Voice();

let callOff: (() => void) | undefined; // drops the call's handlers from the shared socket
let playback: Playback | undefined;
let sender: RTCRtpSender | undefined;
let video: RTCRtpSender | undefined; // the screen share's sender, from joinRoom
let applying: Promise<unknown> = Promise.resolve(); // a sender takes one setParameters at a time
let known: MediaDeviceInfo[] = [];
let poll: ReturnType<typeof setInterval> | undefined;
let statsPoll: ReturnType<typeof setInterval> | undefined;
let releaseLock: (() => void) | undefined;
let silence = MIC_QUIET;
let tally: CallTally | undefined; // this call's summary for Your recent calls, sent when it ends
let posted: Promise<unknown> = Promise.resolve(); // the last summary post, which sign out waits for
const tracks = new Map<string, string>(); // remote track id to user_id, for the per-person stats

// The call code (call-stack.ts) is a chunk of its own: loaded at idle after start (startVoice), on a hover, focus or press
// of Join voice, else at the join.
let stack: typeof import('./call-stack') | undefined;
// The chunk is preloaded at idle, so a failed load at the join is most likely a deploy that replaced its file.
export const CALL_LOAD_FAILED = 'Vesper was updated. Reload the page to join.';
export const loadCall = lazy(() => import('./call-stack').then((m) => (stack = m)));
export const preloadCall = (): Promise<unknown> => loadCall().catch(() => {});

// Push to talk is mute-gated: the track is live only when not muted and (PTT off or key held).
function applyTrack() {
  voice.capture?.setLive(trackLive(voice.muted, voice.settings.pushToTalk.enabled, voice.pttHeld));
}
function sendVoiceState() {
  session.sig?.send({ type: 'set_voice_state', muted: voice.muted, deafened: voice.deafened });
}
export function toggleMute(): void {
  if (voice.forceMuted) return;
  voice.muted = !voice.muted;
  applyTrack();
  sendVoiceState();
}
export function updateSettings(next: AudioSettings): void {
  const prev = voice.settings;
  voice.settings = next;
  saveSettings(storage, next);
  // Only a mic/capture-affecting change can fix a device error.
  if (next.micId !== prev.micId || next.mode !== prev.mode || next.noiseSuppression !== prev.noiseSuppression || next.autoGain !== prev.autoGain)
    voice.deviceError = '';
  if (next.micId !== prev.micId || next.speakerId !== prev.speakerId) voice.notice = '';
  if (!next.pushToTalk.enabled || next.pushToTalk.key !== prev.pushToTalk.key) voice.pttHeld = false;
  voice.capture?.setSettings(next);
  applyTrack();
  if (sender && next.bitrateKbps !== prev.bitrateKbps)
    applyBitrate(sender, next.bitrateKbps).catch((e) => (voice.error = `bitrate change failed: ${e}`));
  if (next.speakerId !== prev.speakerId) {
    const pb = playback;
    pb?.setSink(next.speakerId).catch((e) => {
      if (!next.speakerId || !deviceGone(e)) return void (voice.error = 'could not switch speaker');
      void pb.setSink(null).catch(() => {}); // a failed switch keeps the old output; the notice says default
      speakerGone(pb);
    });
  }
}
// Drops a saved device id that no longer exists, without rebuilding the capture (it already uses the default).
function forget(patch: Partial<AudioSettings>) {
  voice.settings = { ...voice.settings, ...patch };
  saveSettings(storage, voice.settings);
}
function speakerGone(pb: Playback) {
  forget({ speakerId: null });
  if (playback === pb) showNotice(SPEAKER_FALLBACK);
}
// The fallback notice is the call bar's lowest line and goes after a while: it must not hide a live warning for long.
export const NOTICE_MS = 12_000;
let noticeTimer: ReturnType<typeof setTimeout> | undefined;
function showNotice(text: string) {
  clearTimeout(noticeTimer);
  voice.notice = text;
  noticeTimer = setTimeout(() => (voice.notice = ''), NOTICE_MS);
}
/**
 * Retry microphone (the call bar, after the mic ended or with "no sound"): a new mic and capture graph in the same call.
 * With "no sound" the new graph also keeps the browser filter, so no second mic track is opened (see capture.ts).
 */
export async function retryMic(): Promise<void> {
  const cap = voice.capture;
  if (!cap || voice.retrying) return;
  voice.retrying = true;
  try {
    await cap.retry(voice.micWarning === 'mic'); // success or the reason why not both arrive through onModeChange
  } finally {
    voice.retrying = false;
  }
}
export function useOfferedMic(): void {
  updateSettings({ ...voice.settings, micId: voice.offered!.deviceId });
  voice.offered = null;
}
export function dismissOffered(): void {
  voice.offered = null;
}
function onKey(type: 'keydown' | 'keyup') {
  return (e: KeyboardEvent) => {
    const r = pttKeyEvent(voice.pttHeld, {
      type, code: e.code, pttKey: voice.settings.pushToTalk.key, enabled: voice.settings.pushToTalk.enabled,
      typing: isTyping(e.target as Element | null), capturing: false, // Settings stops keys itself while capturing
    });
    if (r.prevent) e.preventDefault();
    if (r.held !== voice.pttHeld) {
      voice.pttHeld = r.held;
      applyTrack();
    }
  };
}

/** Starts the key, blur, visibility and device listeners and the socket subscription. Returns the cleanup, which also ends the call. */
export function startVoice(): () => void {
  voice.settings = loadSettings(storage); // a fresh start, as a new Room mount had
  // The call chunk loads once the app is idle: a join is then warm, and a tab left open over a deploy already has it
  // (the old hashed file is gone after one).
  const ric = 'requestIdleCallback' in window; // not in Safari before 26
  const idle = ric ? window.requestIdleCallback(() => void preloadCall(), { timeout: 5000 }) : setTimeout(() => void preloadCall(), 2000);
  const [kd, ku] = [onKey('keydown'), onKey('keyup')];
  window.addEventListener('keydown', kd);
  window.addEventListener('keyup', ku);
  // keyup never arrives if focus leaves while the key is held, which would leave the mic hot.
  const release = () => { voice.pttHeld = false; applyTrack(); };
  window.addEventListener('blur', release);
  document.addEventListener('visibilitychange', release);
  const stop = watchDevices((list) => {
    // Unplug detection only covers an explicitly selected mic (null = system default is not tracked).
    if (voice.joined && missingDevice(list, voice.settings.micId)) voice.deviceError = DEVICE_MESSAGES.unplugged;
    const fresh = newInputs(known, list);
    if (voice.joined && fresh.length) voice.offered = fresh[0];
    // Heals a pre-permission (unlabelled) list as soon as labels show up.
    if (list.some((d) => d.kind === 'audioinput' && d.deviceId && d.label)) known = list;
  });
  void navigator.mediaDevices.enumerateDevices().then((l) => { if (!known.length) known = l; }, () => {});
  // Sign out: end the call first, so its summary goes out while the session still works.
  const offLogout = onLogout(() => (voice.joined && leave(), posted));
  const offSocket = $effect.root(() => {
    // My own call sounds: one per change of the call's state, none for the state found at start.
    let was: CallSnap | null = null;
    $effect(() => {
      const now = { joined: voice.joined, muted: voice.muted, deafened: voice.deafened, online: session.online };
      const prefs = loadPrefs();
      const cue = callCue(was, now, prefs);
      was = now;
      if (cue) untrack(() => chime(cue, voice.settings.speakerId, prefs.volume));
    });
    // Call events on the shared socket. It is replaced after a reconnect, so this re-subscribes.
    $effect(() => {
      const s = session.sig;
      if (!s) return;
      const offMsg = s.onMessage((m) => {
        if (m.type === 'peer_left') {
          playback?.remove(m.user_id);
          if (voice.speaking.has(m.user_id)) voice.speaking = new Set([...voice.speaking].filter((i) => i !== m.user_id));
        }
        onShareMsg(m);
        const c = callEvent(m, { userId: voice.joined ? voice.userId : '', room: voice.joinedRoom, muted: voice.muted });
        if (c.end) {
          if (c.leave) session.sig?.leave();
          teardown();
          voice.error = c.end;
        } else if (c.forceMuted !== undefined) {
          voice.forceMuted = c.forceMuted;
          if (c.muted !== voice.muted) {
            voice.muted = !!c.muted;
            applyTrack();
          }
        }
      });
      const offClose = s.onClose(() => {
        if (!voice.joined) return;
        teardown();
        voice.error = 'Disconnected from the server.';
      });
      return () => {
        offMsg();
        offClose();
      };
    });
  });
  return () => {
    if (ric) window.cancelIdleCallback(idle);
    else clearTimeout(idle);
    teardown();
    // Nothing from this call may reach the next sign-in (the 4001 close handler can run just before this).
    voice.error = '';
    voice.mode = 'deepfilter';
    flushVolumes(); // now, before the map is emptied for the next account
    voice.volumes = {};
    voice.userId = '';
    voice.joinedBitrate = 64;
    known = [];
    stop();
    offSocket();
    offLogout();
    window.removeEventListener('keydown', kd);
    window.removeEventListener('keyup', ku);
    window.removeEventListener('blur', release);
    document.removeEventListener('visibilitychange', release);
  };
}
export function setLevelling(on: boolean): void {
  voice.levelling = on;
  saveLevelling(on);
  playback?.setLevelling(on);
}
export function toggleDeafen(): void {
  voice.deafened = !voice.deafened;
  playback?.setDeafened(voice.deafened);
  sendVoiceState();
}
// A slider drag sends about 60 values a second: save at most every 250 ms, always including the last one.
let saveTimer: ReturnType<typeof setTimeout> | undefined;
export function setVolume(id: string, v: number): void {
  voice.volumes[id] = v;
  playback?.setVolume(id, v);
  saveTimer ??= setTimeout(flushVolumes, 250);
}
function flushVolumes() {
  if (saveTimer === undefined) return;
  clearTimeout(saveTimer);
  saveTimer = undefined;
  saveVolumes(localStorage, new Map(Object.entries(voice.volumes)));
}
export function leave(): void {
  session.sig?.leave();
  teardown();
  voice.error = '';
}
function teardown() {
  // Whatever ended the call (Leave, a kick, a dropped socket): one summary, to your own list only. A failure loses it.
  if (tally) posted = api('POST', '/api/me/calls', tally.summary(voice.mode, performance.now())).catch(() => {});
  tally = undefined;
  callOff?.();
  callOff = undefined;
  voice.joinedRoom = null;
  clearInterval(poll);
  clearInterval(statsPoll);
  playback?.close();
  playback = undefined;
  releaseLock?.();
  releaseLock = undefined;
  voice.capture?.close();
  voice.capture = undefined;
  sender = undefined;
  endShare();
  video = undefined;
  clearShares();
  voice.pc?.close();
  voice.pc = undefined;
  voice.deviceError = '';
  clearTimeout(noticeTimer);
  voice.notice = '';
  voice.offered = null;
  voice.mic = undefined;
  voice.modeReason = '';
  silence = MIC_QUIET;
  voice.micWarning = null;
  voice.quality = 'unknown';
  voice.diag = null;
  voice.links = {};
  tracks.clear();
  ({ muted: voice.muted, deafened: voice.deafened, forceMuted: voice.forceMuted } = idleCall());
  voice.speaking = new Set();
  voice.joined = false;
}

// Connection quality for the call bar: one stats summary every 2 s while joined.
function pollStats(conn: RTCPeerConnection, m: NonNullable<typeof stack>) {
  let prev: RTCStatsReport | null = null;
  let at = 0;
  statsPoll = setInterval(() => {
    conn.getStats().then((cur) => {
      if (voice.pc !== conn) return; // the call ended while getStats ran
      const now = performance.now();
      voice.diag = m.summarize(prev, cur, (now - at) / 1000, conn.iceConnectionState);
      tally?.poll(voice.diag, cur);
      voice.quality = m.quality(voice.diag);
      const samples = m.linkSamples(prev, cur, tracks);
      samples.set(voice.userId, m.ownSample(voice.diag));
      const muted = (id: string) => session.state.voice.some((p) => p.user_id === id && (p.muted || p.force_muted));
      const live = new Set([...tracks.values(), voice.userId]);
      voice.links = Object.fromEntries([...live].map((id) => [id, m.stepLink(voice.links[id], samples.get(id), muted(id))]));
      prev = cur;
      at = now;
    }, () => {});
  }, 2000);
}

/**
 * Joins `roomId`. Call it from the click handler itself: it creates and resumes the AudioContexts before its first await.
 * Resolves true when the join failed after it started, so the caller moves focus to `voice.error`.
 */
export async function join(roomId: number): Promise<boolean> {
  if (voice.joining) return false; // a second Join (the switcher) while the first waits for the chunk or the mic
  voice.error = '';
  const sig = session.sig;
  if (!sig || !session.online) {
    voice.error = 'Not connected to the server. Wait a moment and try again.';
    return false;
  }
  const room = roomId;
  voice.joining = true;
  ({ muted: voice.muted, deafened: voice.deafened, forceMuted: voice.forceMuted } = idleCall()); // before the join: the server re-sends force_muted after it
  // Create and resume the AudioContexts inside the click gesture, before any await (autoplay policy). Usually the call
  // code is here already (loaded at idle, or by a hover, focus or press of Join voice). If not, make both contexts now and wait for it.
  let code = stack;
  let early: [AudioContext, AudioContext] | undefined;
  if (!code) {
    early = [new AudioContext(), captureContext()];
    early.forEach((c) => void c.resume().catch(() => {}));
    try {
      code = await loadCall();
    } catch {
      early.forEach((c) => void c.close().catch(() => {}));
      voice.error = CALL_LOAD_FAILED;
      voice.joining = false;
      await tick();
      return true;
    }
  }
  playback = new code.Playback(restoreVolumes(localStorage), early?.[0]);
  playback.setLevelling(voice.levelling); // a new Playback each join, so the levelling starts from 0 dB
  // Held for the call's lifetime so the browser does not freeze the tab's audio in the background.
  // The resolver exists up front so teardown releases it even if the lock callback has not run yet.
  const held = new Promise<void>((res) => (releaseLock = res));
  if (navigator.locks) void navigator.locks.request('vesper-call', () => held);
  else console.warn('navigator.locks missing: audio may stop when the tab is backgrounded');
  const pb = playback;
  // A saved speaker that is gone: the context keeps the default output, so forget the id and say so; the join goes on.
  void pb.setSink(voice.settings.speakerId).catch((e) => {
    if (voice.settings.speakerId && deviceGone(e)) speakerGone(pb);
  });
  // Called before any await: it creates its AudioContext (or takes the early one, made inside the click).
  const capturing = code.createCapture(voice.settings, {
    ...(early && { createContext: () => early[1] }),
    startLive: trackLive(false, voice.settings.pushToTalk.enabled, false),
    onMicFallback: () => {
      forget({ micId: null });
      showNotice(MIC_FALLBACK);
    },
  });
  voice.mode = voice.settings.mode;
  let asked = false; // a failure before joinRoom (no mic) never reached the server, so it needs no leave
  try {
    const cap = (voice.capture = await capturing);
    const stream = cap.stream;
    voice.mic = stream;
    // Firefox lists devices without names until the mic is allowed: list them again now.
    void navigator.mediaDevices.enumerateDevices().then((l) => (known = l), () => {});
    voice.volumes = Object.fromEntries(restoreVolumes(localStorage));
    asked = true;
    const r = await code.joinRoom(sig, {
      roomId: room, stream, bitrateKbps: voice.settings.bitrateKbps, dtx: voice.settings.dtx,
      onError: (err) => (voice.error = err.message),
      onRemoteStream: (id, s, kind) => {
        if (kind === 'video') return void (shares.streams = new Map(shares.streams).set(id, s)); // never to playback
        tracks.forEach((u, t) => u === id && tracks.delete(t));
        s.getAudioTracks().forEach((t) => tracks.set(t.id, id));
        playback?.addRemote(id, s);
      },
      onRemoteStreamEnd: (id, kind) => {
        if (kind === 'video') {
          const next = new Map(shares.streams);
          next.delete(id);
          return void (shares.streams = next);
        }
        tracks.forEach((u, t) => u === id && tracks.delete(t));
        if (id in voice.links) voice.links = Object.fromEntries(Object.entries(voice.links).filter(([u]) => u !== id)); // a rejoin starts measuring again
        playback?.remove(id);
      },
    });
    callOff = r.off;
    voice.userId = r.userId;
    voice.pc = r.pc;
    voice.joinedBitrate = voice.settings.bitrateKbps;
    video = r.video;
    resetWatches(); // the server forgot this client's watches: send them again
    playback?.addLocal(r.userId, stream);
    const audioSender = r.pc.getSenders().find((s) => s.track?.kind === 'audio');
    sender = audioSender;
    if (audioSender) cap.setSender(audioSender);
    applyTrack();
    tally = new code.CallTally(performance.now());
    cap.onModeChange((m, reason) => {
      tally?.mode(reason, reason.startsWith('loading'), performance.now());
      voice.mode = m;
      voice.modeReason = reason;
      voice.deviceError = reason.startsWith('microphone lost') ? DEVICE_MESSAGES[classifyMediaError(reason) === 'unknown' ? 'unplugged' : classifyMediaError(reason)] : '';
      applyTrack(); // also after a device error: the new/absent track must still obey mute/PTT
      // The speaking analyser is bound to the old track; re-tap the swapped one.
      playback?.remove(r.userId);
      playback?.addLocal(r.userId, stream);
    });
    poll = setInterval(() => {
      const next = playback?.speaking() ?? new Set<string>();
      if (next.size !== voice.speaking.size || [...next].some((i) => !voice.speaking.has(i))) voice.speaking = next;
      const gated = !trackLive(voice.muted, voice.settings.pushToTalk.enabled, voice.pttHeld);
      const visible = document.visibilityState === 'visible';
      silence = micSilence(silence, performance.now(), voice.capture?.getStats() ?? null, { gated, visible });
      voice.micWarning = silence.show;
    }, 100);
    pollStats(r.pc, code);
    voice.joinedRoom = room;
    voice.joined = true;
    return false;
  } catch (err) {
    if (asked && session.sig === sig) sig.leave(); // the server may have let us in before the failure
    teardown();
    // Before joinRoom only the mic can have failed.
    voice.error = !asked ? micStartError(err) : err instanceof DOMException ? DEVICE_MESSAGES[classifyMediaError(err)] : err instanceof Error ? err.message : String(err);
    await tick();
    return true;
  } finally {
    voice.joining = false;
  }
}

// Screen share. Mine starts with the encoder off; the server's watcher count turns it on and off.
const SHARE_FAILED = 'Could not share your screen.';
const MARKER_MS = 3000;

/** The bitrate to send: the setting, or the suggestion when none is set, never above the browser's ceiling for this size. */
function shareKbps(code: NonNullable<typeof stack>, kbps: number | null, track: MediaStreamTrack): number {
  const { width = 0, height = 0 } = track.getSettings();
  const ceiling = code.ceilingKbps(code.engineOf(navigator.userAgent), width, height);
  return Math.min(kbps ?? code.suggestKbps(null, ceiling), ceiling);
}
function applyMine(): Promise<unknown> {
  const m = shares.mine;
  const s = video;
  if (!m || !s) return applying;
  return (applying = applying.then(() => stack!.applyShare(s, m, m.watchers > 0)).catch(() => {
    if (shares.mine) voice.error = `${SHARE_FAILED} The settings did not apply.`;
  }));
}
/** Ends my share here only. True if there was one. */
function endShare(): boolean {
  const m = shares.mine;
  if (!m) return false;
  shares.mine = null;
  m.track.stop();
  void video?.replaceTrack(null).catch(() => {});
  return true;
}
function onShareMsg(m: ServerMsg) {
  if (m.type === 'watchers' && shares.mine) {
    shares.mine = { ...shares.mine, watchers: m.count };
    void applyMine();
  } else if (m.type === 'peer_share' && m.user_id === voice.userId && !m.share) {
    endShare(); // the server cut my share; its error says why
  } else if (m.type === 'point') {
    const mk = { share: m.user_id, from: m.from, x: m.x, y: m.y, until: Date.now() + MARKER_MS };
    shares.markers = [...shares.markers, mk];
    setTimeout(() => (shares.markers = shares.markers.filter((k) => k !== mk)), MARKER_MS);
  }
}

/** Opens the browser's screen picker and shares the pick. Does nothing if the person cancels it. */
export async function startShare(s: ShareSettings): Promise<void> {
  const sender = video;
  if (!sender || shares.mine) return;
  const code = stack!; // loaded: there is a call
  let track: MediaStreamTrack | null = null;
  try {
    track = await code.capture(s.fps, s.prefer);
    if (!track) return;
    if (video !== sender || shares.mine) return void track.stop(); // the call ended, or another start won, during the picker
    const t = track;
    shares.mine = { track: t, fps: s.fps, kbps: shareKbps(code, s.kbps, t), prefer: s.prefer, watchers: 0 };
    t.addEventListener('ended', () => shares.mine?.track === t && stopShare()); // the browser's own Stop sharing
    await code.applyShare(sender, shares.mine, false);
    await sender.replaceTrack(t);
  } catch (e) {
    if (track) {
      if (shares.mine?.track !== track) return; // the call ended meanwhile
      endShare();
    }
    voice.error = `${SHARE_FAILED} Details: ${e}`;
    return;
  }
  const m = shares.mine;
  if (m?.track === track) session.sig?.send({ type: 'share_start', fps: m.fps, bitrate_kbps: m.kbps });
}
/** New settings for my running share. */
export async function updateShare(s: ShareSettings): Promise<void> {
  const m = shares.mine;
  if (!m) return;
  if (s.fps !== m.fps) void m.track.applyConstraints({ frameRate: s.fps }).catch(() => {});
  shares.mine = { ...m, fps: s.fps, kbps: shareKbps(stack!, s.kbps, m.track), prefer: s.prefer };
  session.sig?.send({ type: 'share_start', fps: s.fps, bitrate_kbps: shares.mine.kbps });
  await applyMine();
}
export function stopShare(): void {
  if (endShare()) session.sig?.send({ type: 'share_stop' });
}
export function watchShare(userId: string, on: boolean): void {
  session.sig?.send({ type: 'watch', user_id: userId, on });
}
export function pointAt(userId: string, x: number, y: number): void {
  session.sig?.send({ type: 'point', user_id: userId, x, y });
}
