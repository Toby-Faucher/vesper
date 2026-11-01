<script lang="ts">
  import type { RoomInfo } from '../lib/gen/RoomInfo';
  import { inRoom } from '../lib/peers';
  import { href } from '../lib/route';
  import { session } from '../lib/session.svelte';
  import { dismissOffered, join, leave, setVolume, toggleDeafen, toggleMute, useOfferedMic, voice } from '../lib/voice.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Chat from './Chat.svelte';
  import RoomHeader from './RoomHeader.svelte';
  // The open room. The shell keeps one instance and changes `room`, so Chat's jump logic sees a room change.
  let { room }: { room: RoomInfo } = $props();

  let errorEl = $state<HTMLElement>();
  const myId = $derived(session.state.me?.user.user_id ?? '');
  const joinedName = $derived(session.state.rooms.find((r) => r.id === voice.joinedRoom)?.name ?? '');
  const peers = $derived(voice.joinedRoom === null ? [] : inRoom(session.state.voice, voice.joinedRoom).filter((p) => p.user_id !== myId));
  const warning = $derived(
    voice.micWarning === 'mic' ? 'No sound from your microphone. Check the input device, its volume, and the mute switch.'
      : voice.micWarning === 'pipeline' ? 'The audio pipeline stopped. Leave and join again.' : '',
  );

  // One call at a time: joining here leaves the call in another room. join() runs synchronously up to its first await,
  // so it is still inside the click gesture.
  async function onJoin() {
    if (voice.joined) leave();
    if (await join(room.id)) errorEl?.focus();
  }
</script>

<RoomHeader {room} inCall={voice.joinedRoom === room.id} joining={voice.joining} onjoin={onJoin} />

<!-- shortcut: the call controls sit here until Task 5's floating call bar replaces them. -->
{#if voice.joined || voice.error || voice.deviceError || voice.offered}
  <div class="flex shrink-0 flex-col gap-2 border-b border-subtle bg-sidebar px-4 py-3 text-sm">
    {#if voice.joined}
      <div class="flex items-center gap-2">
        <span class="min-w-0 truncate text-fg-2">In voice in <a class="text-fg" href={href({ name: 'room', id: voice.joinedRoom! })}>{joinedName}</a></span>
        <span class="ml-auto"></span>
        <Button size="sm" onclick={toggleMute} disabled={voice.forceMuted}>{voice.muted ? 'Unmute' : 'Mute'}</Button>
        <Button size="sm" onclick={toggleDeafen}>{voice.deafened ? 'Undeafen' : 'Deafen'}</Button>
        <Button size="sm" variant="danger" onclick={leave}>Leave</Button>
      </div>
      {#if voice.forceMuted}<p role="status" class="text-warn">A moderator muted you.</p>{/if}
      <p class="text-fg-3" role="status">Noise suppression: {voice.mode}{voice.modeReason ? ` (${voice.modeReason})` : ''}</p>
      <p class="text-warn empty:hidden" role="status">{warning}</p>
      {#each peers as p (p.user_id)}
        <label class="flex items-center gap-3 text-fg-2">
          <span class="w-40 truncate" title={p.name}>{p.name}</span>
          <input type="range" min="0" max="200" aria-label="Volume for {p.name}" value={voice.volumes[p.user_id] ?? 100}
            oninput={(e) => setVolume(p.user_id, +e.currentTarget.value)} />
        </label>
      {/each}
    {/if}
    {#if voice.error}<p class="text-danger" role="alert" tabindex="-1" bind:this={errorEl}>{voice.error}</p>{/if}
    {#if voice.deviceError}<p class="text-warn" role="alert">{voice.deviceError}</p>{/if}
    {#if voice.offered}
      <p class="flex items-center gap-2">
        <span class="min-w-0 truncate">New microphone: {voice.offered.label || voice.offered.deviceId}</span>
        <Button size="sm" onclick={useOfferedMic}>Use it</Button>
        <Button size="sm" variant="ghost" onclick={dismissOffered}>Dismiss</Button>
      </p>
    {/if}
  </div>
{/if}

<Chat roomId={room.id} />
