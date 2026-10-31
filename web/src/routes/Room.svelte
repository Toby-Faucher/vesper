<script lang="ts">
  import { Signaling, joinRoom } from '../lib/signaling';
  import { applyPeerMsg } from '../lib/peers';
  import { Playback } from '../lib/playback';
  import { restoreVolumes, saveVolumes } from '../lib/volume';
  import { createCapture, type Capture } from '../lib/audio/capture';
  import { DEFAULT_AUDIO_SETTINGS } from '../lib/audio/settings';
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
  let mic: MediaStream | undefined;
  let capture: Capture | undefined;
  let mode = $state<DenoiseMode>(DEFAULT_AUDIO_SETTINGS.mode);
  let modeReason = $state('');
  let sig: Signaling | undefined;
  let poll: ReturnType<typeof setInterval> | undefined;
  let releaseLock: (() => void) | undefined;

  function toggleMute() {
    muted = !muted;
    mic?.getAudioTracks().forEach((t) => (t.enabled = !muted));
    sig?.send({ type: 'set_muted', muted });
  }
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
  function teardown() {
    clearInterval(poll);
    playback?.close();
    playback = undefined;
    releaseLock?.();
    releaseLock = undefined;
    capture?.close();
    capture = undefined;
    mic = undefined;
    modeReason = '';
    muted = false;
    deafened = false;
    speaking = new Set();
    joined = false;
  }

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
    // Shortcut: default settings until Task 9 persists them. Called before any await: it creates its AudioContext.
    const capturing = createCapture(DEFAULT_AUDIO_SETTINGS);
    mode = DEFAULT_AUDIO_SETTINGS.mode;
    try {
      const cap = (capture = await capturing);
      const stream = cap.stream;
      mic = stream;
      volumes = Object.fromEntries(restoreVolumes(localStorage));
      const proto = location.protocol === 'https:' ? 'wss' : 'ws';
      sig = new Signaling(`${proto}://${location.host}/ws`);
      const early: ServerMsg[] = []; // peer events that arrive before Joined
      sig.onClose(teardown);
      sig.onMessage((m) => {
        if (m.type === 'peer_left') {
          playback?.remove(m.user_id);
          if (speaking.has(m.user_id)) speaking = new Set([...speaking].filter((i) => i !== m.user_id));
        }
        if (joined) peers = applyPeerMsg(peers, m);
        else early.push(m);
      });
      const r = await joinRoom(sig, {
        room: 'main', name, key, stream, bitrateKbps: 64,
        onError: (err) => (error = err.message),
        onRemoteStream: (s) => playback?.addRemote(s.id, s),
        onRemoteStreamEnd: (id) => playback?.remove(id),
      });
      userId = r.userId;
      playback?.addLocal(r.userId, stream);
      const audioSender = r.pc.getSenders().find((s) => s.track?.kind === 'audio');
      if (audioSender) cap.setSender(audioSender);
      cap.onModeChange((m, reason) => {
        mode = m;
        modeReason = reason;
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
      error = String(err);
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
  {#if joined}
    <div class="flex gap-2">
      <button class="rounded bg-neutral-700 px-3 py-1" onclick={toggleMute}>{muted ? 'Unmute' : 'Mute'}</button>
      <button class="rounded bg-neutral-700 px-3 py-1" onclick={toggleDeafen}>{deafened ? 'Undeafen' : 'Deafen'}</button>
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
