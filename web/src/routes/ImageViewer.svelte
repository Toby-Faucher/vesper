<script lang="ts">
  // The image viewer: one image of the room at fit size, its name, and links to the file. Left and Right move through the
  // images of the loaded part of the room, Esc closes, and focus goes back to the image that opened it (Dialog). Loaded
  // on the first image click. Images come from the same-origin route by id only; the name is text (title, alt, download).
  import type { Attachment } from '../lib/gen/Attachment';
  import * as I from '../lib/ui/icons';
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';

  let { images, at = $bindable(0), onclose }: { images: Attachment[]; at?: number; onclose: () => void } = $props();

  const a = $derived(images[at]);
  const url = $derived(`/api/uploads/${a.id}`);
  const go = (d: number) => (at = (at + d + images.length) % images.length);

  function onKey(e: KeyboardEvent) {
    if (images.length < 2 || e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      go(e.key === 'ArrowLeft' ? -1 : 1);
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<Dialog open title={a.name} wide {onclose}>
  <div class="stage">
    {#key a.id}<img src={url} alt={a.name} />{/key}
  </div>
  {#snippet footer()}
    {#if images.length > 1}
      <div class="mr-auto flex items-center gap-1">
        <IconButton icon={I.chevronLeft} label="Previous image" onclick={() => go(-1)} />
        <IconButton icon={I.chevronRight} label="Next image" onclick={() => go(1)} />
        <span class="pl-1 text-xs text-fg-2 tabular-nums" aria-live="polite">{at + 1} of {images.length}</span>
      </div>
    {/if}
    <a class="link" href={url} target="_blank" rel="noopener noreferrer"><Icon d={I.externalLink} size={16} />Open original</a>
    <a class="link" href={url} download={a.name}><Icon d={I.download} size={16} />Download</a>
  {/snippet}
</Dialog>

<style>
  .stage {
    display: flex;
    height: min(640px, calc(100vh - 220px));
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    background: var(--bg-element);
  }
  .stage img { max-width: 100%; max-height: 100%; object-fit: contain; }
  .link {
    display: inline-flex;
    height: 32px;
    align-items: center;
    gap: 6px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    color: var(--fg);
    font-size: 13px;
    font-weight: 500;
  }
  .link:hover { border-color: var(--line-strong); background: var(--bg-hover); }
</style>
