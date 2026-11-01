<script lang="ts">
  import * as I from '../lib/ui/icons';
  import type { RoomInfo } from '../lib/gen/RoomInfo';
  import { setCollapsed, sidebar } from '../lib/ui/sidebar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  // The open room's name and its one primary action. Pins and Search join it in Tasks 8 and 10.
  let { room, inCall, joining, onjoin }: { room: RoomInfo; inCall: boolean; joining: boolean; onjoin: () => void } = $props();
</script>

<header class="flex h-12 shrink-0 items-center gap-3 border-b border-subtle px-4">
  {#if sidebar.collapsed}
    <span class="-ml-2"><IconButton icon={I.sidebar} label="Show sidebar" tip aria-expanded="false" aria-controls="sidebar" onclick={() => setCollapsed(false)} /></span>
  {/if}
  <h1 class="min-w-0 truncate text-base font-medium text-fg" title={room.name}>{room.name}</h1>
  {#if room.restricted}
    <span class="flex shrink-0 items-center gap-1 text-xs text-fg-2"><Icon d={I.shield} />Restricted</span>
  {/if}
  <div class="ml-auto flex shrink-0 items-center gap-2">
    {#if !inCall}<Button variant="primary" loading={joining} onclick={onjoin}>Join voice</Button>{/if}
  </div>
</header>
