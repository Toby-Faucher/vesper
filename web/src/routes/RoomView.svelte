<script lang="ts">
  import type { RoomInfo } from '../lib/gen/RoomInfo';
  import { join, leave, voice } from '../lib/voice.svelte';
  import Status from '../lib/ui/c/Status.svelte';
  import Chat from './Chat.svelte';
  import RoomHeader from './RoomHeader.svelte';
  // The open room. The shell keeps one instance and changes `room`, so Chat's jump logic sees a room change.
  let { room }: { room: RoomInfo } = $props();

  let errorEl = $state<HTMLElement>();
  // One call at a time: joining here leaves the call in another room. join() runs synchronously up to its first await,
  // so it is still inside the click gesture.
  async function onJoin() {
    if (voice.joined) leave();
    if (await join(room.id)) errorEl?.focus();
  }
</script>

<RoomHeader {room} inCall={voice.joinedRoom === room.id} joining={voice.joining} onjoin={onJoin} />

<!-- A join that failed or a call that ended. While in a call, the call bar shows its own errors. -->
{#if voice.error && !voice.joined}
  <div class="shrink-0 px-4 pt-3" tabindex="-1" bind:this={errorEl}><Status kind="error">{voice.error}</Status></div>
{/if}

<Chat roomId={room.id} />
