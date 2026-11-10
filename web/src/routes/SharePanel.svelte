<script module lang="ts">
  let dock = $state(60); // the panel's part of the width in Dock, in per cent; in memory only
</script>

<script lang="ts">
  import * as I from '../lib/ui/icons';
  import { columns2, eyeOff, galleryVerticalEnd, layoutGrid } from '../lib/share/icons';
  import { session } from '../lib/session.svelte';
  import { fit } from '../lib/share/geometry';
  import { saveLayout, type Layout } from '../lib/share/prefs';
  import type { IconPath } from '../lib/ui/icons';
  import { shares } from '../lib/share/state.svelte';
  import { UI_MESSAGES as M } from '../lib/ui/messages';
  import Button from '../lib/ui/c/Button.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  import ShareTile from './ShareTile.svelte';
  // The screens shared in my call, on the call room's page (design: "Watching"). Open, it puts RoomView's chat beside it (the
  // styles below): a drawer in Grid and Focus, a column after the divider in Dock. Closed, or with every share hidden, it is
  // one row above chat.
  let { ids }: { ids: string[] } = $props();

  const LAYOUTS: [Layout, IconPath][] = [['grid', layoutGrid], ['focus', galleryVerticalEnd], ['dock', columns2]];
  const GAP = 12; // the stage gap, gap-3
  const shown = $derived(ids.filter((id) => !shares.hidden.has(id)));
  const hidden = $derived(ids.filter((id) => shares.hidden.has(id)));
  const open = $derived(shares.panelOpen && shown.length > 0);
  const big = $derived(shares.focus && shown.includes(shares.focus) ? shares.focus : shown[0]);
  const tiles = $derived(shares.layout === 'focus' ? [big] : shown);
  let sw = $state(0), sh = $state(0); // the stage
  const size = $derived(fit(tiles.length, sw, sh, GAP, shares.layout === 'dock' ? 1 : tiles.length));
  // Grid: as many columns of the fitted width as fit. Focus and Dock: one column.
  const columns = $derived(shares.layout === 'grid' ? `repeat(auto-fit, minmax(min(160px, 100%), ${size.width}px))` : `minmax(0, ${size.width}px)`);
  const name = (id: string) => session.state.users[id]?.display_name ?? session.state.voice.find((p) => p.user_id === id)?.name ?? '';

  function setLayout(l: Layout) {
    shares.layout = l;
    saveLayout(l);
  }
  function unhide(id: string) {
    const h = new Set(shares.hidden);
    h.delete(id);
    shares.hidden = h;
  }

  // The divider: drag, or ArrowLeft and ArrowRight in 5% steps.
  const setDock = (v: number) => (dock = Math.min(80, Math.max(20, v)));
  let dragging = $state(false);
  function onmove(e: PointerEvent) {
    if (!dragging) return;
    const r = (e.currentTarget as HTMLElement).parentElement!.getBoundingClientRect();
    setDock(((e.clientX - r.left) / r.width) * 100);
  }
  function onkey(e: KeyboardEvent) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    setDock(dock + (e.key === 'ArrowLeft' ? -5 : 5));
  }
</script>

{#snippet head()}
  <div class="flex min-h-7 shrink-0 flex-wrap items-center gap-x-2 text-xs text-fg-3">
    {#if !shares.panelOpen}
      <span class="flex items-center gap-1.5"><Icon d={I.monitor} />{M.screens(ids.length)}</span>
      <Button variant="ghost" size="sm" onclick={() => (shares.panelOpen = true)}>{M.shareShow}</Button>
    {:else}
      {#if open}<span class="flex items-center gap-1.5"><Icon d={I.monitor} />{M.screens(shown.length)}</span>{/if}
      {#each hidden as id (id)}
        <span class="flex items-center gap-1 text-fg-2"><Icon d={eyeOff} />{M.shareHidden(name(id))}<Button variant="ghost" size="sm" onclick={() => unhide(id)}>{M.shareShow}</Button></span>
      {/each}
      {#if open}
        <span class="ml-auto flex items-center">
          {#each LAYOUTS as [l, icon] (l)}<IconButton {icon} label={M.layouts[l]} tip pressed={shares.layout === l} onclick={() => setLayout(l)} />{/each}
        </span>
        {#if shares.layout !== 'dock'}
          <Button variant="ghost" size="sm" aria-controls="room-chat" aria-expanded={shares.chatOpen} onclick={() => (shares.chatOpen = !shares.chatOpen)}><Icon d={I.room} />{M.shareChat}</Button>
        {/if}
        <Button variant="ghost" size="sm" onclick={() => (shares.panelOpen = false)}>{M.shareHideAll}</Button>
      {/if}
    {/if}
  </div>
{/snippet}

{#if open}
  <section class="panel flex min-h-0 min-w-0 flex-col gap-2 px-4 pt-2" class:flex-1={shares.layout !== 'dock'} class:drawer={shares.layout !== 'dock'} class:chat={shares.chatOpen}
    style:flex={shares.layout === 'dock' ? `0 1 ${dock}%` : null} aria-label={M.screensLabel}>
    {@render head()}
    <div class="grid min-h-0 flex-1 place-content-center gap-3 overflow-hidden" style:grid-template-columns={columns} bind:clientWidth={sw} bind:clientHeight={sh}>
      {#each tiles as id (id)}<ShareTile userId={id} />{/each}
    </div>
    {#if shares.layout === 'focus' && shown.length > 1}
      <div class="flex shrink-0 justify-center gap-3">
        {#each shown.filter((id) => id !== big) as id (id)}<ShareTile userId={id} onpick={() => (shares.focus = id)} />{/each}
      </div>
    {/if}
  </section>
  {#if shares.layout === 'dock'}
    <!-- A focusable separator is a widget (WAI-ARIA window splitter); Svelte's check counts every separator as static. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div class="sep flex-none" role="separator" aria-orientation="vertical" aria-label={M.shareResize} aria-valuenow={Math.round(dock)} aria-valuemin={20} aria-valuemax={80}
      tabindex="0" class:drag={dragging} onkeydown={onkey} onpointermove={onmove} onpointerup={() => (dragging = false)} onpointercancel={() => (dragging = false)}
      onpointerdown={(e) => { dragging = true; e.currentTarget.setPointerCapture(e.pointerId); e.preventDefault(); }}></div>
  {/if}
{:else}
  <div class="shrink-0 px-4 pt-2">{@render head()}</div>
{/if}

<style>
  /* RoomView's row: chat beside the panel. In Grid and Focus a drawer, laid out but hidden while closed, so it keeps its scroll. */
  :global(div):has(> .panel) { flex-direction: row; }
  .panel ~ :global(#room-chat) { min-width: 300px; }
  .drawer ~ :global(#room-chat) { flex: none; width: 400px; border-left: 1px solid var(--line-subtle); }
  .drawer:not(.chat) ~ :global(#room-chat) { position: absolute; inset: 0 0 0 auto; visibility: hidden; }
  /* A 9 px target with a 1 px line, 3 px while dragged, hovered or focused. */
  .sep { width: 9px; padding: 0 4px; cursor: col-resize; touch-action: none; background: var(--line-subtle) content-box; }
  .sep:hover, .drag, .sep:focus-visible { padding: 0 3px; background-color: var(--line-strong); outline: none; }
</style>
