<script lang="ts">
  // The files of one message. An image takes the largest box (360 x 280) before it loads, so a late image never pushes
  // the list; once loaded the box takes the image's own shape, and keepView holds the reader's place. A click opens the
  // viewer. Other files are a card with a Download link. Every file comes from the same-origin route by id; the name
  // is text only (the alt, the title, the download name) and never part of a URL.
  import { tick } from 'svelte';
  import type { Attachment } from '../lib/gen/Attachment';
  import * as I from '../lib/ui/icons';
  import { BOX, fileSize, fit, keepView, typeLabel } from '../lib/ui/attachments';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import Icon from '../lib/ui/c/Icon.svelte';

  let { list, onview }: { list: Attachment[]; onview: (id: string) => void } = $props();

  // Shown size by id, once the image loaded (or failed: a short box with a line of text).
  let shown = $state<Record<string, { w: number; h: number; failed?: boolean }>>({});

  function settle(el: HTMLImageElement, id: string, failed: boolean) {
    if (shown[id]) return;
    void keepView(el, async () => {
      shown[id] = failed ? { w: BOX.w, h: 48, failed } : fit(el.naturalWidth, el.naturalHeight);
      await tick();
    });
  }
</script>

{#each list as a (a.id)}
  {#if a.image}
    {@const s = shown[a.id]}
    <!-- focus() first: Safari does not focus a clicked button, and the viewer gives focus back to the focused element. -->
    <button type="button" class="img" class:failed={s?.failed} aria-label="Open image {a.name}"
      style:width={s ? `${s.w}px` : null} style:aspect-ratio={s ? `${s.w} / ${s.h}` : null} onclick={(e) => { e.currentTarget.focus(); if (!s?.failed) onview(a.id); }}>
      <img src="/api/uploads/{a.id}" alt={a.name} draggable="false"
        onload={(e) => settle(e.currentTarget as HTMLImageElement, a.id, false)} onerror={(e) => settle(e.currentTarget as HTMLImageElement, a.id, true)}
        {@attach (el) => { if (el.complete && el.naturalWidth) settle(el, a.id, false); }} />
      {#if s?.failed}<span class="err">{UI_MESSAGES.imageFailed}</span>{/if}
    </button>
  {:else}
    <div class="card">
      <span class="ic"><Icon d={I.file} /></span>
      <div class="min-w-0 flex-1">
        <p class="truncate text-fg" title={a.name}>{a.name}</p>
        <p class="meta">{[typeLabel(a.mime, a.name), fileSize(a.size)].filter(Boolean).join(', ')}</p>
      </div>
      <a class="mini-act" href="/api/uploads/{a.id}" download={a.name} aria-label="Download {a.name}"><Icon d={I.download} size={16} />Download</a>
    </div>
  {/if}
{/each}

<style>
  .img, .card { width: min(360px, 100%); margin-top: 4px; border-radius: var(--radius); }
  .img {
    position: relative;
    display: block;
    max-width: 100%;
    aspect-ratio: 360 / 280;
    overflow: hidden;
    background: var(--bg-element);
    cursor: zoom-in;
  }
  .img img { display: block; width: 100%; height: 100%; object-fit: contain; }
  .img:focus-visible { outline-offset: 2px; }
  .img.failed { cursor: default; }
  .img.failed img { visibility: hidden; }
  .err { position: absolute; inset: 0; display: flex; align-items: center; padding: 0 12px; color: var(--fg-2); font-size: 12px; line-height: 16px; }
  .card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 8px 8px 12px;
    border: 1px solid var(--line-subtle);
    background: var(--bg-raised);
    font-size: 13px;
    line-height: 20px;
  }
  .ic { display: inline-flex; flex-shrink: 0; color: var(--fg-2); }
  .meta { color: var(--fg-2); font-size: 12px; line-height: 16px; }
</style>
