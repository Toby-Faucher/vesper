<script lang="ts">
  // The viewer: one image or video of the room at fit size, its name, and links to the file. Left and Right move through the
  // images and videos of the loaded part of the room, Esc closes, and focus goes back to what opened it (Dialog). A video
  // plays with the browser's controls; leaving it (another item, Esc) removes it, which stops it. Loaded on the first open.
  // Files come from the same-origin route by id only; the name is text (title, alt, label, download).
  import type { Attachment } from '../lib/gen/Attachment';
  import * as I from '../lib/ui/icons';
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';

  let { items, at = $bindable(0), onclose }: { items: Attachment[]; at?: number; onclose: () => void } = $props();

  const a = $derived(items[at]);
  const url = $derived(`/api/uploads/${a.id}`);
  const go = (d: number) => (at = (at + d + items.length) % items.length);

  function onKey(e: KeyboardEvent) {
    if (items.length < 2 || e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      go(e.key === 'ArrowLeft' ? -1 : 1);
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<Dialog open title={a.name} wide {onclose}>
  <div class="stage">
    {#key a.id}
      {#if a.kind === 'video'}
        <!-- svelte-ignore a11y_media_has_caption: user uploads come with no captions -->
        <video src={url} controls autoplay playsinline aria-label="Video {a.name}"></video>
      {:else}<img src={url} alt={a.name} />{/if}
    {/key}
  </div>
  {#snippet footer()}
    {#if items.length > 1}
      <div class="mr-auto flex items-center gap-1">
        <IconButton icon={I.chevronLeft} label="Previous" onclick={() => go(-1)} />
        <IconButton icon={I.chevronRight} label="Next" onclick={() => go(1)} />
        <span class="pl-1 text-xs text-fg-2 tabular-nums" aria-live="polite">{at + 1} of {items.length}</span>
      </div>
    {/if}
    <a class="mini-act lined" href={url} target="_blank" rel="noopener noreferrer"><Icon d={I.externalLink} size={16} />Open original</a>
    <a class="mini-act lined" href={url} download={a.name}><Icon d={I.download} size={16} />Download</a>
  {/snippet}
</Dialog>

<style>
  /* The stage takes the image's or video's size (up to the dialog and the window), so a small one does not float in a band. */
  .stage { display: flex; min-height: 96px; align-items: center; justify-content: center; border-radius: var(--radius); background: var(--bg-element); }
  .stage img, .stage video { max-width: 100%; max-height: min(640px, calc(100vh - 220px)); }
</style>
