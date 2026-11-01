<script lang="ts">
  import { setContext, type Snippet } from 'svelte';
  import { menuMove } from '../menu';
  import Popover from './Popover.svelte';
  // A Popover with role="menu" of MenuItems. Roving focus: Up, Down, Home, End and typeahead by first letter (menu.ts).
  // Enter or Space activates the item (native button), Esc closes (the popover), Tab closes and goes back to the anchor.
  let { anchor, open, onclose, label, children }: { anchor: HTMLElement | null; open: boolean; onclose: () => void; label: string; children: Snippet } =
    $props();
  setContext('vesper.menu', { close: () => onclose() });

  const items = (el: HTMLElement) => [...el.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:enabled')];

  function keydown(e: KeyboardEvent) {
    if (e.key === 'Tab') {
      e.preventDefault();
      onclose();
      return;
    }
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const list = items(e.currentTarget as HTMLElement);
    const next = menuMove(e.key, list.indexOf(document.activeElement as HTMLButtonElement), list.map((b) => b.textContent ?? ''));
    if (next === null) return;
    e.preventDefault();
    list[next].focus();
  }
</script>

<Popover {anchor} {open} {onclose} {label} role="menu" onkeydown={keydown} onopen={(el) => (items(el)[0] ?? el).focus()}>
  {@render children()}
</Popover>
