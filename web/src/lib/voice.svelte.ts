// The voice call, moved from Room.svelte so it outlives the screen that shows it.
import { tick, untrack } from 'svelte';
import { callEvent, idleCall } from './call';
import { session } from './session.svelte';
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
let known: MediaDeviceInfo[] = [];
let poll: ReturnType<typeof setInterval> | undefined;
let statsPoll: ReturnType<typeof setInterval> | undefined;
let releaseLock: (() => void) | undefined;
let silence = MIC_QUIET;
const tracks = new Map<string, string>(); // remote track id to user_id, for the per-person stats

// The call code (call-stack.ts) is a chunk of its own: loaded on a hover, focus or press of Join voice, else at the join.
let stack: typeof import('./call-stack') | undefined;
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
/** Retry microphone (the call bar, after the mic ended): a new mic and capture graph in the same call. */
export async function retryMic(): Promise<void> {
  const cap = voice.capture;
  if (!cap || voice.retrying) return;
  voice.retrying = true;
  try {
    await cap.retry(); // success or the reason why not both arrive through onModeChange
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
  // code is here already (a hover, focus or press of Join voice loaded it). If not, make both contexts now and wait for it.
  let code = stack;
  let early: [AudioContext, AudioContext] | undefined;
  if (!code) {
    early = [new AudioContext(), captureContext()];
    early.forEach((c) => void c.resume().catch(() => {}));
    try {
      code = await loadCall();
    } catch {
      early.forEach((c) => void c.close().catch(() => {}));
      voice.error = 'Could not load the call. Check your connection and try again.';
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
      onRemoteStream: (id, s) => {
        tracks.forEach((u, t) => u === id && tracks.delete(t));
        s.getAudioTracks().forEach((t) => tracks.set(t.id, id));
        playback?.addRemote(id, s);
      },
      onRemoteStreamEnd: (id) => {
        tracks.forEach((u, t) => u === id && tracks.delete(t));
        if (id in voice.links) voice.links = Object.fromEntries(Object.entries(voice.links).filter(([u]) => u !== id)); // a rejoin starts measuring again
        playback?.remove(id);
      },
    });
    callOff = r.off;
    voice.userId = r.userId;
    voice.pc = r.pc;
    voice.joinedBitrate = voice.settings.bitrateKbps;
    playback?.addLocal(r.userId, stream);
    const audioSender = r.pc.getSenders().find((s) => s.track?.kind === 'audio');
    sender = audioSender;
    if (audioSender) cap.setSender(audioSender);
    applyTrack();
    cap.onModeChange((m, reason) => {
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
