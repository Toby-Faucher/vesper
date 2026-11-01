<script lang="ts">
  import type { Snippet } from 'svelte';
  import { enter } from '../motion';
  import { place } from '../place';
  // A popover="auto" layer next to `anchor`: light dismiss (outside click, Esc) by the platform, placed by place().
  // On open it focuses itself (or what `onopen` picks); on close focus goes back to the anchor when it was inside.
  // It sets aria-expanded on the anchor. Arrow keys are left to the children.
  let {
    anchor,
    open,
    onclose,
    label,
    side = 'bottom',
    role = 'dialog',
    onkeydown,
    onopen,
    children,
  }: {
    anchor: HTMLElement | null;
    open: boolean;
    onclose: () => void;
    label: string;
    side?: 'bottom' | 'top' | 'right';
    role?: 'dialog' | 'menu';
    onkeydown?: (e: KeyboardEvent) => void;
    onopen?: (el: HTMLElement) => void;
    children: Snippet;
  } = $props();
  let el: HTMLElement;
  let downOnAnchor = false;

  const shown = () => el.matches(':popover-open');
  function refocus() {
    const a = document.activeElement;
    if (anchor && (!a || a === document.body || el.contains(a))) anchor.focus();
  }

  $effect(() => {
    if (!anchor) return;
    anchor.setAttribute('aria-expanded', String(open));
    if (open && !shown()) {
      // source: the anchor counts as the invoker, so a click on it toggles instead of light-dismiss and reopen.
      el.showPopover({ source: anchor });
      // shortcut: placed once on open; a scroll or resize while open does not move it. Fine for menus and cards.
      const r = el.getBoundingClientRect();
      const p = place(anchor.getBoundingClientRect(), { w: r.width, h: r.height }, { w: innerWidth, h: innerHeight }, side);
      el.style.left = `${p.x}px`;
      el.style.top = `${p.y}px`;
      enter(el, 'pop');
      (onopen ?? ((e) => e.focus()))(el);
    } else if (!open && shown()) {
      el.hidePopover();
      refocus();
    }
  });

  // A browser without the `source` option dismisses on pointerdown on the anchor and the anchor's click then reopens it.
  // So a dismiss that starts on the anchor waits for the click: the owner toggles open off, or we report the close after it.
  $effect(() => {
    const a = anchor;
    if (!a) return;
    const down = () => {
      if (!open) return;
      downOnAnchor = true;
      addEventListener('pointerup', () => setTimeout(() => {
        downOnAnchor = false;
        if (open && !shown()) onclose();
      }), { once: true });
    };
    a.addEventListener('pointerdown', down);
    return () => a.removeEventListener('pointerdown', down);
  });

  function toggle(e: ToggleEvent) {
    if (e.newState !== 'closed' || !open) return;
    refocus();
    if (!downOnAnchor) onclose();
  }
</script>

<div bind:this={el} popover="auto" {role} aria-label={label} tabindex="-1" ontoggle={toggle} {onkeydown}>
  {@render children()}
</div>

<style>
  div {
    position: fixed;
    inset: auto;
    min-width: 176px;
    max-width: 320px;
    margin: 0;
    padding: 4px;
    border: 1px solid var(--line-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-raised);
    color: var(--fg);
    font-size: 13px;
    line-height: 20px;
    box-shadow: var(--shadow-float);
  }
  div:focus-visible { outline-offset: -2px; }
</style>
