<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { joinRoom } from '../lib/signaling';
  import { inRoom } from '../lib/peers';
  import { session, wsUrl } from '../lib/session.svelte';
  import { Playback } from '../lib/playback';
  import { restoreVolumes, saveVolumes } from '../lib/volume';
  import { createCapture, type Capture } from '../lib/audio/capture';
  import { loadSettings, saveSettings, type AudioSettings } from '../lib/audio/settings';
  import { DEVICE_MESSAGES, applyBitrate, classifyMediaError, missingDevice, newInputs, trackLive, watchDevices } from '../lib/devices';
  import { pttKeyEvent } from '../lib/audio/ptt';
  import { MIC_QUIET, micSilence, type MicWarning } from '../lib/audio/mic-silence';
  import Settings from './Settings.svelte';
  import Diagnostics from './Diagnostics.svelte';
  import type { DenoiseMode } from '../lib/audio/mode';

  let roomId = $state(session.state.rooms[0]?.id ?? 0); // the room picked in the list
  let joinedRoom = $state<number | null>(null);
  let joining = $state(false);
  let forceMuted = $state(false);
  let callOff: (() => void) | undefined; // drops the call's handlers from the shared socket
  let joined = $state(false);
  let error = $state('');
  let userId = $state('');
  let muted = $state(false);
  let deafened = $state(false);
  let volumes = $state<Record<string, number>>({});
  let speaking = $state<Set<string>>(new Set());
  let playback: Playback | undefined;
  let mic = $state<MediaStream>();
  let capture = $state.raw<Capture>();
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
  let poll: ReturnType<typeof setInterval> | undefined;
  let releaseLock: (() => void) | undefined;
  let silence = MIC_QUIET;
  let micWarning = $state<MicWarning>(null);

  // Push to talk is mute-gated: the track is live only when not muted and (PTT off or key held).
  function applyTrack() {
    capture?.setLive(trackLive(muted, settings.pushToTalk.enabled, pttDown));
  }
  function sendVoiceState() {
    session.sig?.send({ type: 'set_voice_state', muted, deafened });
  }
  function toggleMute() {
    if (forceMuted) return;
    muted = !muted;
    applyTrack();
    sendVoiceState();
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
      teardown();
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
    sendVoiceState();
  }
  function setVolume(id: string, v: number) {
    volumes[id] = v;
    playback?.setVolume(id, v);
    const m = new Map(Object.entries(volumes));
    saveVolumes(localStorage, m);
  }
  function leave() {
    session.sig?.leave();
    teardown();
    error = '';
  }
  function teardown() {
    callOff?.();
    callOff = undefined;
    joinedRoom = null;
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
    deviceError = '';
    offered = null;
    mic = undefined;
    modeReason = '';
    silence = MIC_QUIET;
    micWarning = null;
    muted = false;
    deafened = false;
    speaking = new Set();
    joined = false;
  }

  // Call events on the shared socket. It is replaced after a reconnect, so this re-subscribes.
  $effect(() => {
    const s = session.sig;
    if (!s) return;
    const offMsg = s.onMessage((m) => {
      if (m.type === 'peer_left') {
        playback?.remove(m.user_id);
        if (speaking.has(m.user_id)) speaking = new Set([...speaking].filter((i) => i !== m.user_id));
        // Our own id: the call ended (another tab took it, a moderator kicked us, or access was lost).
        if (joined && m.user_id === userId) {
          teardown();
          error = 'Your call ended. You may have joined from another tab, or a moderator removed you.';
        }
      } else if (m.type === 'force_muted') {
        forceMuted = m.muted;
        if (m.muted && !muted) {
          muted = true; // lifting a force-mute does not unmute
          applyTrack();
        }
      }
    });
    const offClose = s.onClose(() => {
      if (!joined) return;
      teardown();
      error = 'Disconnected from the server.';
    });
    return () => {
      offMsg();
      offClose();
    };
  });

  // Keep the picked room valid, and end the call if its room is no longer in the list.
  $effect(() => {
    const rooms = session.state.rooms;
    untrack(() => {
      if (rooms.length && !rooms.some((r) => r.id === roomId)) roomId = rooms[0].id;
      if (joinedRoom !== null && !rooms.some((r) => r.id === joinedRoom)) {
        session.sig?.leave();
        teardown();
        error = 'You no longer have access to that room.';
      }
    });
  });

  const myId = $derived(session.state.me?.user.user_id ?? '');
  const peers = $derived(inRoom(session.state.voice, joinedRoom ?? roomId).filter((p) => !(joined && p.user_id === myId)));
  const joinedName = $derived(session.state.rooms.find((r) => r.id === joinedRoom)?.name ?? '');

  async function join(e: Event) {
    e.preventDefault();
    error = '';
    const sig = session.sig;
    if (!sig || !session.online) {
      error = 'Not connected to the server. Wait a moment and try again.';
      return;
    }
    const room = roomId;
    joining = true;
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
      const r = await joinRoom(sig, {
        roomId: room, stream, bitrateKbps: settings.bitrateKbps, dtx: settings.dtx,
        onError: (err) => (error = err.message),
        onRemoteStream: (s) => playback?.addRemote(s.id, s),
        onRemoteStreamEnd: (id) => playback?.remove(id),
      });
      callOff = r.off;
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
        const gated = !trackLive(muted, settings.pushToTalk.enabled, pttDown);
        const visible = document.visibilityState === 'visible';
        silence = micSilence(silence, performance.now(), capture?.getStats() ?? null, { gated, visible });
        micWarning = silence.show;
      }, 100);
      joinedRoom = room;
      joined = true;
    } catch (err) {
      if (session.sig === sig) sig.leave(); // the server may have let us in before the failure
      teardown();
      error = err instanceof DOMException ? DEVICE_MESSAGES[classifyMediaError(err)] : err instanceof Error ? err.message : String(err);
    } finally {
      joining = false;
    }
  }
</script>

<main class="mx-auto max-w-sm space-y-4 p-6">
  <div class="flex items-center gap-2 text-sm">
    <span class="flex-1">Signed in as {session.state.me?.user.display_name}</span>
    <button class="rounded bg-neutral-800 px-3 py-1" onclick={() => session.logout()}>Log out</button>
  </div>
  {#if session.notice}
    <p role="alert" class="text-amber-400">
      {session.notice}
      <button class="rounded bg-neutral-700 px-2" onclick={() => session.connect()}>Reconnect</button>
    </p>
  {:else if !session.online}
    <p role="status" class="text-neutral-400">Connecting to the server...</p>
  {/if}
  {#if !joined}
    <form onsubmit={join} class="space-y-3">
      <select class="w-full rounded bg-neutral-900 p-2" aria-label="Room" bind:value={roomId}>
        {#each session.state.rooms as r (r.id)}<option value={r.id}>{r.name}</option>{/each}
      </select>
      <button class="w-full rounded bg-neutral-700 p-2" disabled={joining || !session.state.rooms.length}>
        {joining ? 'Joining...' : 'Join'}
      </button>
    </form>
  {/if}
  {#if error}<p class="text-red-400" role="alert">{error}</p>{/if}
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
    <Diagnostics {pc} {capture} wsUrl={wsUrl()} {mode} status={joined ? 'joined' : 'not joined'} />
  {/if}
  {#if joined}
    <p>In {joinedName}</p>
    <div class="flex gap-2">
      <button class="rounded bg-neutral-700 px-3 py-1" onclick={toggleMute} disabled={forceMuted}>{muted ? 'Unmute' : 'Mute'}</button>
      <button class="rounded bg-neutral-700 px-3 py-1" onclick={toggleDeafen}>{deafened ? 'Undeafen' : 'Deafen'}</button>
      <button class="ml-auto rounded bg-red-900 px-3 py-1" onclick={leave}>Leave</button>
    </div>
    {#if forceMuted}<p role="status" class="text-amber-400">A moderator muted you.</p>{/if}
    <p class="rounded p-1 {speaking.has(userId) ? 'ring-2 ring-green-500' : ''}">You{muted ? ' (muted)' : ''}{deafened ? ' (deafened)' : ''}</p>
    <p class="text-sm text-neutral-400" role="status">Noise suppression: {mode}{modeReason ? ` (${modeReason})` : ''}</p>
    <p class="text-sm text-amber-400" role="status">
      {micWarning === 'mic' ? 'No sound from your microphone. Check the input device, its volume, and the mute switch.'
        : micWarning === 'pipeline' ? 'The audio pipeline stopped. Leave and join again.' : ''}
    </p>
  {/if}
  <ul class="space-y-1">
    {#each peers as p (p.user_id)}
      <li class="flex items-center gap-2 rounded p-1 {speaking.has(p.user_id) ? 'ring-2 ring-green-500' : ''}">
        <span class="flex-1">{p.name}{p.muted ? ' (muted)' : ''}{p.deafened ? ' (deafened)' : ''}</span>
        <input type="range" min="0" max="200" aria-label="Volume for {p.name}"
          value={volumes[p.user_id] ?? 100}
          oninput={(e) => setVolume(p.user_id, +e.currentTarget.value)} />
      </li>
    {/each}
  </ul>
</main>
