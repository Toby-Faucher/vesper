<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { inRoom } from '../lib/peers';
  import { session, wsUrl } from '../lib/session.svelte';
  import { dismissOffered, join, leave, setVolume, startVoice, toggleDeafen, toggleMute, updateSettings, useOfferedMic, voice } from '../lib/voice.svelte';
  import Settings from './Settings.svelte';
  import Diagnostics from './Diagnostics.svelte';
  import Chat from './Chat.svelte';

  let roomId = $state(session.state.rooms[0]?.id ?? 0); // the room picked in the list
  let errorEl = $state<HTMLElement>();
  let showSettings = $state(false);
  let showDiag = $state(false);

  onMount(startVoice);

  // Keep the picked room valid. A call whose room left the list ends in callEvent.
  $effect(() => {
    const rooms = session.state.rooms;
    untrack(() => {
      if (rooms.length && !rooms.some((r) => r.id === roomId)) roomId = rooms[0].id;
    });
  });

  const myId = $derived(session.state.me?.user.user_id ?? '');
  const peers = $derived(inRoom(session.state.voice, voice.joinedRoom ?? roomId).filter((p) => !(voice.joined && p.user_id === myId)));
  // The room list shows unread counts, then mentions: "general (3, 1 @)".
  const badge = (id: number) => {
    const u = session.state.unread[id];
    return u?.unread ? ` (${u.unread}${u.mentions ? `, ${u.mentions} @` : ''})` : '';
  };
  const joinedName = $derived(session.state.rooms.find((r) => r.id === voice.joinedRoom)?.name ?? '');

  async function onJoin(e: Event) {
    e.preventDefault();
    if (await join(roomId)) errorEl?.focus(); // join runs synchronously up to its first await, inside the gesture
  }
</script>

<main class="mx-auto grid max-w-5xl gap-6 p-6 md:grid-cols-[20rem_1fr]">
<div class="space-y-4">
  <div class="flex items-center gap-2 text-sm">
    <span class="flex-1">Signed in as {session.state.me?.user.display_name}</span>
    <button class="rounded bg-hover px-3 py-1" onclick={() => session.logout()}>Log out</button>
  </div>
  {#if session.notice}
    <p role="alert" class="text-warn">
      {session.notice}
      <button class="rounded bg-selected px-2" onclick={() => session.connect()}>Reconnect</button>
    </p>
  {:else if !session.online}
    <p role="status" class="text-fg-3">Connecting to the server...</p>
  {/if}
  <select class="w-full rounded bg-element p-2" aria-label="Room" bind:value={roomId}>
    {#each session.state.rooms as r (r.id)}<option value={r.id}>{r.name}{badge(r.id)}</option>{/each}
  </select>
  {#if !voice.joined}
    <form onsubmit={onJoin} class="space-y-3">
      <button class="w-full rounded bg-selected p-2" disabled={voice.joining || !session.state.rooms.length}>
        {voice.joining ? 'Joining...' : 'Join'}
      </button>
    </form>
  {/if}
  {#if voice.error}<p class="text-danger" role="alert" tabindex="-1" bind:this={errorEl}>{voice.error}</p>{/if}
  {#if voice.deviceError}<p class="text-warn" role="alert">{voice.deviceError}</p>{/if}
  {#if voice.offered}
    <p class="text-sm">
      New microphone: {voice.offered.label || voice.offered.deviceId}
      <button class="rounded bg-selected px-2" onclick={useOfferedMic}>Use it</button>
      <button class="px-2 text-fg-3" onclick={dismissOffered}>Dismiss</button>
    </p>
  {/if}
  <button class="rounded bg-hover px-3 py-1 text-sm" onclick={() => (showSettings = !showSettings)}>
    {showSettings ? 'Hide settings' : 'Audio settings'}
  </button>
  {#if showSettings}
    <Settings settings={voice.settings} joinedBitrate={voice.joinedBitrate} joined={voice.joined} onchange={updateSettings} liveStream={voice.mic} sinkSupported={'setSinkId' in AudioContext.prototype} />
  {/if}
  <button class="rounded bg-hover px-3 py-1 text-sm" onclick={() => (showDiag = !showDiag)}>
    {showDiag ? 'Hide diagnostics' : 'Diagnostics'}
  </button>
  {#if showDiag}
    <Diagnostics pc={voice.pc} capture={voice.capture} wsUrl={wsUrl()} mode={voice.mode} status={voice.joined ? 'joined' : 'not joined'} />
  {/if}
  {#if voice.joined}
    <p>In {joinedName}</p>
    <div class="flex gap-2">
      <button class="rounded bg-selected px-3 py-1" onclick={toggleMute} disabled={voice.forceMuted}>{voice.muted ? 'Unmute' : 'Mute'}</button>
      <button class="rounded bg-selected px-3 py-1" onclick={toggleDeafen}>{voice.deafened ? 'Undeafen' : 'Deafen'}</button>
      <button class="ml-auto rounded bg-danger-solid text-on-danger px-3 py-1" onclick={leave}>Leave</button>
    </div>
    {#if voice.forceMuted}<p role="status" class="text-warn">A moderator muted you.</p>{/if}
    <p class="rounded p-1 {voice.speaking.has(voice.userId) ? 'speaking ring-2 ring-ok' : ''}">You{voice.muted ? ' (muted)' : ''}{voice.deafened ? ' (deafened)' : ''}</p>
    <p class="text-sm text-fg-3" role="status">Noise suppression: {voice.mode}{voice.modeReason ? ` (${voice.modeReason})` : ''}</p>
    <p class="text-sm text-warn" role="status">
      {voice.micWarning === 'mic' ? 'No sound from your microphone. Check the input device, its volume, and the mute switch.'
        : voice.micWarning === 'pipeline' ? 'The audio pipeline stopped. Leave and join again.' : ''}
    </p>
  {/if}
  <ul class="space-y-1">
    {#each peers as p (p.user_id)}
      <li class="flex items-center gap-2 rounded p-1 {voice.speaking.has(p.user_id) ? 'speaking ring-2 ring-ok' : ''}">
        <span class="flex-1">{p.name}{p.muted ? ' (muted)' : ''}{p.deafened ? ' (deafened)' : ''}</span>
        <input type="range" min="0" max="200" aria-label="Volume for {p.name}"
          value={voice.volumes[p.user_id] ?? 100}
          oninput={(e) => setVolume(p.user_id, +e.currentTarget.value)} />
      </li>
    {/each}
  </ul>
</div>
{#if roomId}<Chat bind:roomId />{/if}
</main>
