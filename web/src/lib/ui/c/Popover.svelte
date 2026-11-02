<script lang="ts">
  import type { Snippet } from 'svelte';
  import { enter, ghost } from '../motion';
  import { place } from '../place';
  // A popover="auto" layer next to `anchor`: light dismiss (outside click, Esc) by the platform, placed by place().
  // On open it focuses itself (or what `onopen` picks); on close focus goes back to the anchor when it was inside.
  // It sets aria-haspopup (the role), aria-controls and aria-expanded on the anchor. Arrow keys are left to the children.
  // `above`: opening upwards, it also stays above this element (a card from the call bar clears the composer).
  let {
    anchor,
    open,
    onclose,
    label,
    side = 'bottom',
    above = null,
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
    above?: Element | null;
    role?: 'dialog' | 'menu';
    onkeydown?: (e: KeyboardEvent) => void;
    onopen?: (el: HTMLElement) => void;
    children: Snippet;
  } = $props();
  const id = $props.id();
  let el: HTMLElement;
  let downOnAnchor = false;

  const shown = () => el.matches(':popover-open');
  function refocus() {
    const a = document.activeElement;
    // el is null when the popover was removed while open: its toggle event comes after the unmount.
    if (anchor && (!a || a === document.body || el?.contains(a))) anchor.focus();
  }

  $effect(() => {
    const a = anchor;
    if (!a) return;
    a.setAttribute('aria-haspopup', role);
    a.setAttribute('aria-controls', id);
    return () => ['aria-haspopup', 'aria-controls', 'aria-expanded'].forEach((n) => a.removeAttribute(n));
  });

  $effect(() => {
    if (!anchor) return;
    anchor.setAttribute('aria-expanded', String(open));
    if (open && !shown()) {
      // source: the anchor counts as the invoker, so a click on it toggles instead of light-dismiss and reopen.
      el.showPopover({ source: anchor });
      // shortcut: placed once on open; a scroll or resize while open does not move it. Fine for menus and cards.
      // A card opened downwards still grows down when its content grows; nothing sits under one today.
      const r = el.getBoundingClientRect();
      const a = anchor.getBoundingClientRect();
      const box = above && side === 'top' ? { left: a.left, right: a.right, bottom: a.bottom, top: Math.min(a.top, above.getBoundingClientRect().top) } : a;
      const p = place(box, { w: r.width, h: r.height }, { w: innerWidth, h: innerHeight }, side);
      el.style.left = `${p.x}px`;
      // Opened upwards it hangs from its bottom edge, so content that grows while open (the card's stats line wrapping on
      // the first poll) pushes it up, not down over its anchor and the composer.
      const up = p.side === 'top';
      el.style.top = up ? '' : `${p.y}px`;
      el.style.bottom = up ? `${innerHeight - p.y - r.height}px` : '';
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
    let t: ReturnType<typeof setTimeout> | undefined;
    const up = () => (t = setTimeout(() => {
      downOnAnchor = false;
      if (open && !shown()) onclose();
    }));
    const down = () => {
      if (!open) return;
      downOnAnchor = true;
      addEventListener('pointerup', up, { once: true });
    };
    a.addEventListener('pointerdown', down);
    return () => {
      a.removeEventListener('pointerdown', down);
      removeEventListener('pointerup', up);
      clearTimeout(t); // the anchor's click can unmount us (a second click on a name closes its card) before this runs
    };
  });

  function toggle(e: ToggleEvent) {
    // Removed while open (the owner closed it, or another name's card replaced it): the browser still sends this event,
    // a task after the unmount. Nothing to report then: an onclose now would close what the owner shows next.
    if (e.newState !== 'closed' || !open || !el) return;
    refocus();
    if (!downOnAnchor) onclose();
  }
</script>

<div bind:this={el} {id} popover="auto" {role} aria-label={label} tabindex="-1" ontoggle={toggle} onbeforetoggle={(e) => e.newState === 'closed' && ghost(el, 'pop')} {onkeydown}>
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
