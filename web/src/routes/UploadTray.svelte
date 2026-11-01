<script lang="ts">
  // The files waiting above the composer's text box: one row each with the name, the size and the state (a progress bar
  // while it uploads, Waiting in the queue, the reason when it failed), Retry for a failure that may pass, and Cancel
  // (aborts) or Remove. An image picked from this device shows a small preview from an object URL, revoked when the row
  // goes. The name is text only. "Attached: <name>" (the prefix for screen readers) marks a file that is ready to send.
  import type { Tray, TrayItem } from '../lib/ui/tray.svelte';
  import * as I from '../lib/ui/icons';
  import { fileSize } from '../lib/ui/attachments';
  import Icon from '../lib/ui/c/Icon.svelte';

  let { tray }: { tray: Tray } = $props();

  const PREVIEW = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']; // never SVG
  const PREFIX = { queued: 'Waiting: ', uploading: 'Uploading: ', done: 'Attached: ', failed: 'Failed: ' };
  const pct = (i: TrayItem) => Math.round(i.progress * 100);
  const preview = (file: File) => (el: HTMLImageElement) => {
    const u = URL.createObjectURL(file);
    el.src = u;
    return () => URL.revokeObjectURL(u);
  };
</script>

<ul class="tray" aria-label="Files to send">
  {#each tray.items as i (i.key)}
    {@const busy = i.state === 'queued' || i.state === 'uploading'}
    <li class="item" class:failed={i.state === 'failed'}>
      {#if PREVIEW.includes(i.file.type)}
        <img class="thumb" alt="" {@attach preview(i.file)} />
      {:else}
        <span class="thumb"><Icon d={I.file} size={16} /></span>
      {/if}
      <span class="name" title={i.file.name}><span class="sr-only">{PREFIX[i.state]}</span>{i.file.name}</span>
      <span class="size">{fileSize(i.file.size)}</span>
      <span class="state">
        {#if i.state === 'uploading'}
          <span class="bar" role="progressbar" aria-label="Upload progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct(i)}>
            <span style:width="{pct(i)}%"></span>
          </span>
          <span class="pct" aria-hidden="true">{pct(i)}%</span>
        {:else if i.state === 'queued'}
          <span class="text-fg-2">Waiting</span>
        {:else if i.state === 'done'}
          <span class="flex items-center gap-1 text-fg-2"><Icon d={I.check} size={16} />Ready</span>
        {:else if i.retry}
          <button type="button" class="act" aria-label="Retry {i.file.name}" onclick={() => tray.retry(i.key)}><Icon d={I.retry} size={16} />Retry</button>
        {/if}
      </span>
      <button type="button" class="x" aria-label="{busy ? 'Cancel' : 'Remove'} {i.file.name}" title={busy ? 'Cancel' : 'Remove'}
        onclick={() => tray.remove(i.key)}><Icon d={I.x} size={16} /></button>
      {#if i.state === 'failed'}<p class="err" role="alert">{i.error}</p>{/if}
    </li>
  {/each}
</ul>

<style>
  .tray { padding: 4px; border-bottom: 1px solid var(--line-subtle); }
  .item {
    display: grid;
    grid-template-columns: 24px minmax(0, auto) auto 1fr auto;
    align-items: center;
    column-gap: 8px;
    min-height: 32px;
    padding: 4px 4px 4px 8px;
    border-radius: var(--radius);
    font-size: 13px;
    line-height: 20px;
  }
  .item:hover { background: var(--bg-hover); }
  .thumb {
    display: inline-flex;
    width: 24px;
    height: 24px;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    background: var(--bg-raised);
    color: var(--fg-2);
    object-fit: cover;
  }
  .name { overflow: hidden; color: var(--fg); text-overflow: ellipsis; white-space: nowrap; }
  .size { color: var(--fg-2); font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .state { display: flex; min-width: 0; align-items: center; justify-content: flex-end; gap: 8px; font-size: 12px; line-height: 16px; }
  .bar { position: relative; width: 160px; height: 4px; overflow: hidden; border-radius: 2px; background: var(--line); }
  .bar > span { position: absolute; inset: 0 auto 0 0; background: var(--accent); transition: width var(--dur-1) linear; }
  .pct { width: 32px; color: var(--fg-2); font-variant-numeric: tabular-nums; text-align: right; }
  .act, .x {
    display: inline-flex;
    height: 24px;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    color: var(--fg-2);
  }
  .act { gap: 4px; padding: 0 8px; color: var(--fg); font-size: 12px; font-weight: 500; }
  .x { width: 24px; }
  .act:hover, .x:hover { background: var(--bg-selected); color: var(--fg); }
  .err { grid-column: 2 / -1; color: var(--danger-fg); font-size: 12px; line-height: 16px; }
  @media (prefers-reduced-motion: reduce) {
    .bar > span { transition: none; }
  }
</style>
