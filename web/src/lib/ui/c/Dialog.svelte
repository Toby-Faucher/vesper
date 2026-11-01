<script lang="ts">
  import * as I from '../icons';
  import { onMount, type Snippet } from 'svelte';
  import { enter } from '../motion';
  import IconButton from './IconButton.svelte';
  // A modal on native <dialog>: showModal() traps focus and makes the page inert, Esc closes. A click on the backdrop closes
  // too. Focus goes back to the element that had it when the dialog opened. Initial focus is the first control in the body
  // (the Close button comes last in the DOM, drawn top right). Dialogs stack: Esc closes the top one, and each gives focus
  // back to its own opener. The page behind does not scroll while any modal is open (the :has rule below counts them all).
  // `id` lets the opener point aria-controls at the dialog. `bare` (the switcher): the title is for screen readers only,
  // there is no Close button (Esc closes), the body has no padding, it opens without motion, wider and near the top.
  // `wide` (the image viewer): as wide as the content, up to 1040 px.
  let {
    open,
    title,
    onclose,
    children,
    footer,
    id: given,
    bare = false,
    wide = false,
  }: { open: boolean; title: string; onclose: () => void; children: Snippet; footer?: Snippet; id?: string; bare?: boolean; wide?: boolean } = $props();
  const uid = $props.id();
  const id = $derived(given ?? uid);
  let el: HTMLDialogElement; // null once destroyed (bind:this), when a late close event can still arrive
  let opener: HTMLElement | null = null;
  let downOnBackdrop = false;

  $effect(() => {
    if (open && !el.open) {
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      el.showModal();
      shown = true;
      if (!bare) enter(el, 'pop');
    } else if (!open && el.open) shut();
  });

  // The close event comes a task after close(), when a new open may already have happened: so every close here runs
  // closed() at once, `shown` makes it run once per open, and a close event while open again is stale and ignored. Esc
  // (cancel) closes the same way, so a key pressed right after it (Ctrl+K again) sees the dialog as closed.
  let shown = false;
  function shut() {
    el.close();
    closed();
  }
  // Removed while open (the person card re-keyed, a sign out): no close runs, so give focus back here. Not onclose: the
  // owner is going away.
  onMount(() => {
    const d = el; // bind:this is null by the time this runs
    return () => {
      if (!shown) return;
      shown = false;
      d.close(); // leave the top layer first, else the page is still inert and the opener cannot take focus
      opener?.focus();
    };
  });
  function closed() {
    if (!shown) return;
    shown = false;
    opener?.focus();
    opener = null;
    if (open) onclose(); // closed by Esc or the backdrop: tell the owner
  }
</script>

<dialog
  bind:this={el}
  {id}
  class:bare
  class:wide
  aria-labelledby="{id}-title"
  onclose={() => el?.open || closed()}
  oncancel={(e) => { e.preventDefault(); shut(); }}
  onpointerdown={(e) => (downOnBackdrop = e.target === el)}
  onclick={(e) => downOnBackdrop && e.target === el && shut()}
>
  <div class="relative flex max-h-[inherit] flex-col">
    <h2 id="{id}-title" class={bare ? 'sr-only' : 'px-5 pt-4 pr-14 pb-2 text-lg font-medium text-fg [overflow-wrap:anywhere]'}>{title}</h2>
    <div class="min-h-0 overflow-y-auto text-sm text-fg-2 {bare ? 'flex flex-col' : 'px-5 pt-2 pb-5'}">{@render children()}</div>
    {#if footer}<div class="flex justify-end gap-2 border-t border-subtle px-5 py-3">{@render footer()}</div>{/if}
    {#if !bare}<div class="absolute top-3 right-3"><IconButton icon={I.x} label="Close" onclick={shut} /></div>{/if}
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
  /* As wide as its content (the viewer's image), from the normal width up to 1040 px. */
  dialog.wide { width: fit-content; min-width: min(440px, calc(100vw - 32px)); max-width: min(1040px, calc(100vw - 64px)); }
  dialog.bare { width: min(600px, calc(100vw - 32px)); max-height: min(520px, calc(100vh - 128px)); margin-top: 96px; }
  /* A modal makes the page inert but wheel and touch still scroll it. Gone with the last open modal, also on destroy. */
  :global(html:has(dialog:modal)) { overflow: hidden; }
  dialog::backdrop { background: color-mix(in oklab, var(--bg-app) 72%, transparent); }
</style>
