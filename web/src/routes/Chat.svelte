<script lang="ts">
  // The chat panel for the picked room: the timeline and the composer (search is a dialog, SearchDialog.svelte).
  // Files dragged over the panel show the drop overlay and go to the composer's upload tray. Text and links dragged
  // around are left alone; a file dropped anywhere else in the app is ignored instead of opening in the tab.
  import { onMount, untrack } from 'svelte';
  import Composer from './Composer.svelte';
  import Timeline from './Timeline.svelte';
  import { ApiFailure, errorText } from '../lib/api';
  import type { Message } from '../lib/gen/Message';
  import * as I from '../lib/ui/icons';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import { Tray } from '../lib/ui/tray.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';

  let { roomId }: { roomId: number } = $props();

  let error = $state('');
  let replyTo = $state<Message | null>(null);
  let composer = $state<Composer>();
  const tray = new Tray();
  let drag = $state(0); // dragenter minus dragleave over the panel, for file drags only

  // A new room: an action error and the files waiting to be sent belong to the old one.
  $effect(() => {
    void roomId;
    error = '';
    untrack(() => tray.clear()); // reads the tray: only the room change may re-run this
  });
  onMount(() => () => tray.clear());

  const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files');
  function onDragOver(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = 'copy';
  }
  function onDrop(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    drag = 0;
    tray.add(roomId, [...e.dataTransfer!.files]);
    composer?.focus();
  }

  /** Runs an action and shows its error (`conflict` replaces the table's text for a 409). True when it worked. */
  async function act(f: () => Promise<unknown>, conflict?: string): Promise<boolean> {
    error = '';
    try {
      await f();
      return true;
    } catch (e) {
      error = conflict && e instanceof ApiFailure && e.status === 409 ? conflict : errorText(e);
      return false;
    }
  }

  // An empty room invites the first message: focus the composer, unless the reader already moved focus into the pane
  // (a key press landing there must not be taken away when the load finishes).
  function sayHello() {
    const a = document.activeElement;
    if (!a || a === document.body || !a.closest('main')) composer?.focus();
  }

  function reply(m: Message) {
    replyTo = m;
    composer?.focus();
  }
</script>

<svelte:window ondragover={(e) => hasFiles(e) && e.preventDefault()} ondrop={(e) => { if (hasFiles(e)) { e.preventDefault(); drag = 0; } }}
  ondragend={() => (drag = 0)} />

<section class="relative flex min-h-0 flex-1 flex-col pt-3" aria-label="Chat"
  ondragenter={(e) => { if (hasFiles(e)) { e.preventDefault(); drag++; } }}
  ondragleave={(e) => { if (hasFiles(e)) drag = Math.max(0, drag - 1); }}
  ondragover={onDragOver} ondrop={onDrop}>
  <Timeline {roomId} {act} onreply={reply} onempty={sayHello} />

  {#if error}<p class="shrink-0 px-5 pt-1 text-xs text-danger" role="alert">{error}</p>{/if}
  <Composer {roomId} {tray} bind:this={composer} bind:replyTo />

  {#if drag}
    <div class="drop" aria-hidden="true"><Icon d={I.paperclip} />Drop to attach</div>
  {/if}
  <span class="sr-only" aria-live="polite">{drag ? UI_MESSAGES.dropFiles : ''}</span>
</section>

<style>
  /* Plain and still: a dashed amber edge over the panel, no motion. It lets drag events through to the panel. */
  .drop {
    position: absolute;
    inset: 8px 16px 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: 2px dashed var(--accent-line);
    border-radius: var(--radius-lg);
    background: color-mix(in oklab, var(--bg-app) 88%, transparent);
    color: var(--fg);
    font-size: 14px;
    font-weight: 500;
    line-height: 20px;
    pointer-events: none;
  }
</style>
