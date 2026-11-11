<script lang="ts">
  import type { RoomInfo } from '../lib/gen/RoomInfo';
  import { REJOINING, join, leave, loadCall, voice } from '../lib/voice.svelte';
  import { session } from '../lib/session.svelte';
  import { sharersIn } from '../lib/peers';
  import Status from '../lib/ui/c/Status.svelte';
  import { UI_MESSAGES } from '../lib/ui/messages-first';
  import RoomHeader from './RoomHeader.svelte';
  // The open room. The shell keeps one instance and changes `room`, so Chat's jump logic sees a room change.
  let { room }: { room: RoomInfo } = $props();
  // Chat and the markdown renderer load on the first room open, not with sign in or Settings. One promise per RoomView,
  // so a room change keeps the same Chat instance.
  const chat = import('./Chat.svelte');

  let errorEl = $state<HTMLElement>();
  // One call at a time: joining here leaves the call in another room. join() runs synchronously up to its first await,
  // so it is still inside the click gesture.
  async function onJoin() {
    if (voice.joined) leave();
    if (await join(room.id)) errorEl?.focus();
  }
  const ids = $derived(voice.joinedRoom === room.id ? sharersIn(session.state.voice, room.id, voice.userId) : []);
</script>

<RoomHeader {room} inCall={voice.joinedRoom === room.id} joining={voice.joining} onjoin={onJoin} />

<!-- A join that failed or a call that ended. While in a call, the call bar shows its own errors. -->
{#if voice.error && !voice.joined}
  <div class="shrink-0 px-4 pt-3" tabindex="-1" bind:this={errorEl}><Status kind="error">{voice.error}</Status></div>
{:else if voice.rejoin !== null}
  <div class="shrink-0 px-4 pt-3"><Status kind="warn">{REJOINING}</Status></div>
{/if}

<!-- An open share panel sets this row's layout (SharePanel.svelte): chat beside the screens, as a drawer or a column. -->
<div class="relative flex min-h-0 flex-1 flex-col">
  <!-- The share panel and its tiles are in the call chunk, loaded before a join can finish. -->
  {#if ids.length}{#await loadCall() then m}<m.SharePanel {ids} />{/await}{/if}
  <!-- One element in every layout, so chat keeps its scroll and draft. -->
  <div id="room-chat" class="flex min-h-0 min-w-0 flex-1 flex-col">
    {#await chat}
      <p class="flex-1 px-4 pt-3 text-sm text-fg-3">{UI_MESSAGES.chatLoading}</p>
    {:then m}
      <!-- Shell drops `room` once it leaves the list (archived, made private) or the route changes, and a room load that
           settles after that still reads this id: it gets 0 (no room) then, not a throw. -->
      <m.default roomId={room?.id ?? 0} />
    {:catch}
      <p role="alert" class="flex-1 px-4 pt-3 text-sm text-danger">{UI_MESSAGES.chatFailed}</p>
    {/await}
  </div>
</div>
