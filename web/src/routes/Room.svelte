<script lang="ts">
  import { onMount } from 'svelte';
  import { Signaling, joinRoom } from '../lib/signaling';
  import { applyPeerMsg } from '../lib/peers';
  import { Playback } from '../lib/playback';
  import { restoreVolumes, saveVolumes } from '../lib/volume';
  import { createCapture, type Capture } from '../lib/audio/capture';
  import { loadSettings, saveSettings, type AudioSettings } from '../lib/audio/settings';
  import { DEVICE_MESSAGES, applyBitrate, classifyMediaError, missingDevice, newInputs, trackLive, watchDevices } from '../lib/devices';
  import { pttKeyEvent } from '../lib/audio/ptt';
  import Settings from './Settings.svelte';
  import Diagnostics from './Diagnostics.svelte';
  import type { DenoiseMode } from '../lib/audio/mode';
  import type { Peer } from '../lib/gen/Peer';
  import type { ServerMsg } from '../lib/gen/ServerMsg';

  let name = $state('');
  let key = $state('');
  let peers = $state<Peer[]>([]);
  let joined = $state(false);
  let error = $state('');
  let userId = $state('');
  let muted = $state(false);
  let deafened = $state(false);
  let volumes = $state<Record<string, number>>({});
  let speaking = $state<Set<string>>(new Set());
  let playback: Playback | undefined;
  let mic = $state<MediaStream>();
  let capture: Capture | undefined;
  // Accessing localStorage itself throws when storage is blocked.
  const storage = (() => { try { return localStorage; } catch { return null; } })();
  let settings = $state<AudioSettings>(loadSettings(storage));
  let joinedBitrate = $state(64); // set at join
  let showSettings = $state(false);
  let showDiag = $state(false);
  let pc = $state.raw<RTCPeerConnection>();
  let deviceError = $state('');
  let offered = $state<MediaDeviceInfo | null>(null);
  let sender: RTCRtpSender | undefined;
  let pttDown = false;
  let known: MediaDeviceInfo[] = [];
  let mode = $state<DenoiseMode>('deepfilter');
  let modeReason = $state('');
  let sig: Signaling | undefined;
  let poll: ReturnType<typeof setInterval> | undefined;
  let releaseLock: (() => void) | undefined;

  // Push to talk is mute-gated: the track is live only when not muted and (PTT off or key held).
  function applyTrack() {
    capture?.setLive(trackLive(muted, settings.pushToTalk.enabled, pttDown));
  }
  function toggleMute() {
    muted = !muted;
    applyTrack();
    sig?.send({ type: 'set_muted', muted });
  }
  function onSettings(next: AudioSettings) {
    const prev = settings;
    settings = next;
    saveSettings(storage, next);
    // Only a mic/capture-affecting change can fix a device error.
    if (next.micId !== prev.micId || next.mode !== prev.mode || next.noiseSuppression !== prev.noiseSuppression || next.autoGain !== prev.autoGain)
      deviceError = '';
    if (!next.pushToTalk.enabled || next.pushToTalk.key !== prev.pushToTalk.key) pttDown = false;
    capture?.setSettings(next);
    applyTrack();
    if (sender && next.bitrateKbps !== prev.bitrateKbps)
      applyBitrate(sender, next.bitrateKbps).catch((e) => (error = `bitrate change failed: ${e}`));
    if (next.speakerId !== prev.speakerId)
      playback?.setSink(next.speakerId).catch(() => (error = 'could not switch speaker'));
  }
  function onKey(type: 'keydown' | 'keyup') {
    return (e: KeyboardEvent) => {
      const r = pttKeyEvent(pttDown, {
        type, code: e.code, pttKey: settings.pushToTalk.key, enabled: settings.pushToTalk.enabled,
        typing: !!(e.target as HTMLElement | null)?.matches?.('input,textarea,select'), capturing: false, // Settings stops keys itself while capturing
      });
      if (r.prevent) e.preventDefault();
      if (r.held !== pttDown) {
        pttDown = r.held;
        applyTrack();
      }
    };
  }
  onMount(() => {
    const [kd, ku] = [onKey('keydown'), onKey('keyup')];
    window.addEventListener('keydown', kd);
    window.addEventListener('keyup', ku);
    // keyup never arrives if focus leaves while the key is held, which would leave the mic hot.
    const release = () => { pttDown = false; applyTrack(); };
    window.addEventListener('blur', release);
    document.addEventListener('visibilitychange', release);
    const stop = watchDevices((list) => {
      // Unplug detection only covers an explicitly selected mic (null = system default is not tracked).
      if (joined && missingDevice(list, settings.micId)) deviceError = DEVICE_MESSAGES.unplugged;
      const fresh = newInputs(known, list);
      if (joined && fresh.length) offered = fresh[0];
      // Heals a pre-permission (unlabelled) list as soon as labels show up.
      if (list.some((d) => d.kind === 'audioinput' && d.deviceId && d.label)) known = list;
    });
    void navigator.mediaDevices.enumerateDevices().then((l) => { if (!known.length) known = l; }, () => {});
    return () => {
      stop();
      window.removeEventListener('keydown', kd);
      window.removeEventListener('keyup', ku);
      window.removeEventListener('blur', release);
      document.removeEventListener('visibilitychange', release);
    };
  });
  function toggleDeafen() {
    deafened = !deafened;
    playback?.setDeafened(deafened);
  }
  function setVolume(id: string, v: number) {
    volumes[id] = v;
    playback?.setVolume(id, v);
    const m = new Map(Object.entries(volumes));
    saveVolumes(localStorage, m);
  }
  function leave() {
    sig?.leave();
    sig = undefined;
    teardown();
    error = '';
  }
  function teardown() {
    clearInterval(poll);
    playback?.close();
    playback = undefined;
    releaseLock?.();
    releaseLock = undefined;
    capture?.close();
    capture = undefined;
    sender = undefined;
    pc?.close();
    pc = undefined;
    peers = [];
    deviceError = '';
    offered = null;
    mic = undefined;
    modeReason = '';
    muted = false;
    deafened = false;
    speaking = new Set();
    joined = false;
  }

  const wsUrl = () => `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`;

  async function join(e: Event) {
    e.preventDefault();
    error = '';
    muted = false;
    deafened = false;
    // Create and resume the AudioContext inside the click gesture, before any await.
    playback = new Playback(restoreVolumes(localStorage));
    // Held for the call's lifetime so the browser does not freeze the tab's audio in the background.
    // The resolver exists up front so teardown releases it even if the lock callback has not run yet.
    const held = new Promise<void>((res) => (releaseLock = res));
    if (navigator.locks) void navigator.locks.request('vesper-call', () => held);
    else console.warn('navigator.locks missing: audio may stop when the tab is backgrounded');
    void playback.setSink(settings.speakerId).catch(() => {});
    // Called before any await: it creates its AudioContext.
    const capturing = createCapture(settings, { startLive: trackLive(false, settings.pushToTalk.enabled, false) });
    mode = settings.mode;
    try {
      const cap = (capture = await capturing);
      const stream = cap.stream;
      mic = stream;
      volumes = Object.fromEntries(restoreVolumes(localStorage));
      const mine = (sig = new Signaling(wsUrl()));
      const early: ServerMsg[] = []; // peer events that arrive before Joined
      mine.onClose((byClient) => {
        if (sig !== mine) return; // an old socket closing late must not tear down a newer call
        const lostCall = joined && !byClient; // server restart, same-name takeover, ping timeout
        teardown();
        if (lostCall) error = 'Disconnected from the server';
      });
      sig.onMessage((m) => {
        if (m.type === 'peer_left') {
          playback?.remove(m.user_id);
          if (speaking.has(m.user_id)) speaking = new Set([...speaking].filter((i) => i !== m.user_id));
        }
        if (joined) peers = applyPeerMsg(peers, m);
        else early.push(m);
      });
      const r = await joinRoom(sig, {
        room: 'main', name, key, stream, bitrateKbps: settings.bitrateKbps, dtx: settings.dtx,
        onError: (err) => (error = err.message),
        onRemoteStream: (s) => playback?.addRemote(s.id, s),
        onRemoteStreamEnd: (id) => playback?.remove(id),
      });
      userId = r.userId;
      pc = r.pc;
      joinedBitrate = settings.bitrateKbps;
      playback?.addLocal(r.userId, stream);
      const audioSender = r.pc.getSenders().find((s) => s.track?.kind === 'audio');
      sender = audioSender;
      if (audioSender) cap.setSender(audioSender);
      applyTrack();
      cap.onModeChange((m, reason) => {
        mode = m;
        modeReason = reason;
        deviceError = reason.startsWith('microphone lost') ? DEVICE_MESSAGES[classifyMediaError(reason) === 'unknown' ? 'unplugged' : classifyMediaError(reason)] : '';
        applyTrack(); // also after a device error: the new/absent track must still obey mute/PTT
        // The speaking analyser is bound to the old track; re-tap the swapped one.
        playback?.remove(r.userId);
        playback?.addLocal(r.userId, stream);
      });
      poll = setInterval(() => {
        const next = playback?.speaking() ?? new Set<string>();
        if (next.size !== speaking.size || [...next].some((i) => !speaking.has(i))) speaking = next;
      }, 100);
      peers = early.reduce(applyPeerMsg, r.peers);
      joined = true;
    } catch (err) {
      sig?.close();
      teardown();
      error = err instanceof DOMException ? DEVICE_MESSAGES[classifyMediaError(err)] : String(err);
    }
  }
</script>

<main class="mx-auto max-w-sm space-y-4 p-6">
  {#if !joined}
    <form onsubmit={join} class="space-y-3">
      <input class="w-full rounded bg-neutral-900 p-2" placeholder="Name" bind:value={name} required />
      <input class="w-full rounded bg-neutral-900 p-2" placeholder="Key" type="password" bind:value={key} required />
      <button class="w-full rounded bg-neutral-700 p-2">Join</button>
    </form>
  {/if}
  {#if error}<p class="text-red-400">{error}</p>{/if}
  {#if deviceError}<p class="text-amber-400" role="alert">{deviceError}</p>{/if}
  {#if offered}
    <p class="text-sm">
      New microphone: {offered.label || offered.deviceId}
      <button class="rounded bg-neutral-700 px-2" onclick={() => { onSettings({ ...settings, micId: offered!.deviceId }); offered = null; }}>Use it</button>
      <button class="px-2 text-neutral-400" onclick={() => (offered = null)}>Dismiss</button>
    </p>
  {/if}
  <button class="rounded bg-neutral-800 px-3 py-1 text-sm" onclick={() => (showSettings = !showSettings)}>
    {showSettings ? 'Hide settings' : 'Audio settings'}
  </button>
  {#if showSettings}
    <Settings {settings} {joinedBitrate} {joined} onchange={onSettings} liveStream={mic} sinkSupported={'setSinkId' in AudioContext.prototype} />
  {/if}
  <button class="rounded bg-neutral-800 px-3 py-1 text-sm" onclick={() => (showDiag = !showDiag)}>
    {showDiag ? 'Hide diagnostics' : 'Diagnostics'}
  </button>
  {#if showDiag}
    <Diagnostics {pc} wsUrl={wsUrl()} joinKey={key} {mode} status={joined ? 'joined' : 'not joined'} />
  {/if}
  {#if joined}
    <div class="flex gap-2">
      <button class="rounded bg-neutral-700 px-3 py-1" onclick={toggleMute}>{muted ? 'Unmute' : 'Mute'}</button>
      <button class="rounded bg-neutral-700 px-3 py-1" onclick={toggleDeafen}>{deafened ? 'Undeafen' : 'Deafen'}</button>
      <button class="ml-auto rounded bg-red-900 px-3 py-1" onclick={leave}>Leave</button>
    </div>
    <p class="rounded p-1 {speaking.has(userId) ? 'ring-2 ring-green-500' : ''}">You{muted ? ' (muted)' : ''}</p>
    <p class="text-sm text-neutral-400" role="status">Noise suppression: {mode}{modeReason ? ` (${modeReason})` : ''}</p>
  {/if}
  <ul class="space-y-1">
    {#each peers as p (p.user_id)}
      <li class="flex items-center gap-2 rounded p-1 {speaking.has(p.user_id) ? 'ring-2 ring-green-500' : ''}">
        <span class="flex-1">{p.name}{p.muted ? ' (muted)' : ''}</span>
        <input type="range" min="0" max="200" aria-label="Volume for {p.name}"
          value={volumes[p.user_id] ?? 100}
          oninput={(e) => setVolume(p.user_id, +e.currentTarget.value)} />
      </li>
    {/each}
  </ul>
</main>
