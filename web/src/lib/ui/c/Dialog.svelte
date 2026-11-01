<script lang="ts">
  import * as I from '../icons';
  import type { Snippet } from 'svelte';
  import { enter } from '../motion';
  import IconButton from './IconButton.svelte';
  // A modal on native <dialog>: showModal() traps focus and makes the page inert, Esc closes. A click on the backdrop closes
  // too. Focus goes back to the element that had it when the dialog opened. Initial focus is the first control in the body
  // (the Close button comes last in the DOM, drawn top right). Dialogs stack: Esc closes the top one, and each gives focus
  // back to its own opener. The page behind does not scroll while any modal is open (the :has rule below counts them all).
  // `id` lets the opener point aria-controls at the dialog.
  let {
    open,
    title,
    onclose,
    children,
    footer,
    id: given,
  }: { open: boolean; title: string; onclose: () => void; children: Snippet; footer?: Snippet; id?: string } = $props();
  const uid = $props.id();
  const id = $derived(given ?? uid);
  let el: HTMLDialogElement;
  let opener: HTMLElement | null = null;
  let downOnBackdrop = false;

  $effect(() => {
    if (open && !el.open) {
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      el.showModal();
      enter(el, 'pop');
    } else if (!open && el.open) el.close();
  });

  function closed() {
    opener?.focus();
    opener = null;
    if (open) onclose(); // closed by Esc or the backdrop: tell the owner
  }
</script>

<dialog
  bind:this={el}
  {id}
  aria-labelledby="{id}-title"
  onclose={closed}
  onpointerdown={(e) => (downOnBackdrop = e.target === el)}
  onclick={(e) => downOnBackdrop && e.target === el && el.close()}
>
  <div class="relative flex max-h-[inherit] flex-col">
    <h2 id="{id}-title" class="px-5 pt-4 pr-14 pb-2 text-lg font-medium text-fg">{title}</h2>
    <div class="min-h-0 overflow-y-auto px-5 pt-2 pb-5 text-sm text-fg-2">{@render children()}</div>
    {#if footer}<div class="flex justify-end gap-2 border-t border-subtle px-5 py-3">{@render footer()}</div>{/if}
    <div class="absolute top-3 right-3"><IconButton icon={I.x} label="Close" onclick={() => el.close()} /></div>
  </div>
</dialog>

<style>
  dialog {
    width: min(440px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    margin: auto; /* Tailwind's preflight zeroes margins, which undoes the UA centring */
    padding: 0;
    border: 1px solid var(--line-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-raised);
    color: var(--fg);
    box-shadow: var(--shadow-float);
  }
  /* A modal makes the page inert but wheel and touch still scroll it. Gone with the last open modal, also on destroy. */
  :global(html:has(dialog:modal)) { overflow: hidden; }
  dialog::backdrop { background: color-mix(in oklab, var(--bg-app) 72%, transparent); }
</style>
