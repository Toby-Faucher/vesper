<script lang="ts">
  import * as I from '../lib/ui/icons';
  import type { RoomInfo } from '../lib/gen/RoomInfo';
  import { setCollapsed, sidebar } from '../lib/ui/sidebar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import { layers } from '../lib/ui/layers.svelte';
  import { session } from '../lib/session.svelte';
  // The open room's name, Pins, Search and its one primary action. The pins list (with the Markdown
  // renderer) loads on the first click, so it stays out of the first load.
  let { room, inCall, joining, onjoin }: { room: RoomInfo; inCall: boolean; joining: boolean; onjoin: () => void } = $props();
  let pinsEl = $state<HTMLButtonElement | null>(null);
  let pins = $state<Promise<typeof import('./PinsPopover.svelte')> | null>(null);
  let pinsOpen = $state(false);
  let pinsFailed = false;
  function togglePins() {
    if (pinsFailed) [pins, pinsFailed] = [null, false]; // the chunk did not load: try again
    pins ??= import('./PinsPopover.svelte').catch((e) => {
      [pinsOpen, pinsFailed] = [false, true];
      throw e;
    });
    pinsOpen = !pinsOpen;
  }
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
    <span class="pins"><IconButton icon={I.pin} label="Pins" tip bind:el={pinsEl} onclick={togglePins} /></span>
    {#if pins}
      {#await pins then m}
        <m.default roomId={room.id} anchor={pinsEl} open={pinsOpen} onclose={() => (pinsOpen = false)} />
      {:catch}
        <span class="sr-only" role="alert">{UI_MESSAGES.pinsFailed}</span>
      {/await}
    {/if}
    <IconButton icon={I.search} label="Search" tip aria-haspopup="dialog" aria-controls="search" aria-expanded={layers.search} onclick={() => (layers.search = true)} />
    {#if !inCall}
      <Button variant="primary" loading={joining} onclick={onjoin} disabled={!session.online}
        data-tip={session.online ? undefined : UI_MESSAGES.offlineAction} data-tip-end>Join voice</Button>
    {/if}
  </div>
</header>

<style>
  .pins { display: contents; }
  .pins :global([aria-expanded='true']) { background: var(--bg-selected); color: var(--fg); }
</style>
