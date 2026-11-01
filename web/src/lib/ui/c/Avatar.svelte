<script lang="ts">
  import { avatarSlot, initials } from '../avatar';
  import { ICONS } from '../icons';
  // Initials on the user's slot colour. Speaking draws the ring (2 px gap in --bg-sidebar, 2 px slot colour), toggled
  // instantly apart from a --dur-1 opacity change. Mute or deafen shows a glyph at the bottom right at 32 and 40 px; at
  // 20 px it would be a 10 px speck, so it is left out and the row shows the state after the name (an Icon plus the word,
  // as the gallery's sidebar mock does). Decorative: the row next to it carries the name and the state in words.
  let {
    user,
    size,
    speaking = false,
    muted = false,
    deafened = false,
  }: {
    user: { user_id: string; display_name: string; accent: string };
    size: 20 | 32 | 40;
    speaking?: boolean;
    muted?: boolean;
    deafened?: boolean;
  } = $props();
  const slot = $derived(avatarSlot(user));
  // One letter at 20 px: two would not fit at the 12 px floor.
  const text = $derived([...new Intl.Segmenter().segment(initials(user.display_name) || '?')].slice(0, size === 20 ? 1 : 2).map((s) => s.segment).join(''));
  const glyph = $derived(size === 20 ? null : deafened ? 'headphones-off' : muted ? 'mic-off' : null);
</script>

<span
  class="av s{size}"
  data-speaking={speaking || undefined}
  style:--c="var(--avatar-{slot})"
  aria-hidden="true"
>
  {text}
  {#if glyph}
    <span class="glyph"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><path d={ICONS[glyph]} /></svg></span>
  {/if}
</span>

<style>
  .av {
    position: relative;
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: var(--c);
    color: var(--on-avatar);
    font-weight: 500;
    user-select: none;
  }
  .av::before {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: 50%;
    box-shadow: 0 0 0 2px var(--bg-sidebar), 0 0 0 4px var(--c);
    opacity: 0;
    transition: opacity var(--dur-1) linear;
  }
  .av[data-speaking]::before { opacity: 1; }
  .s20 { width: 20px; height: 20px; font-size: 12px; line-height: 16px; }
  .s32 { width: 32px; height: 32px; font-size: 13px; line-height: 20px; }
  .s40 { width: 40px; height: 40px; font-size: 16px; line-height: 24px; }
  .glyph {
    position: absolute;
    right: -3px;
    bottom: -3px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: var(--bg-sidebar);
    color: var(--danger-fg);
  }
  .s32 .glyph { width: 18px; height: 18px; right: -4px; bottom: -4px; }
  .s32 svg { width: 12px; height: 12px; }
  .s40 .glyph { width: 20px; height: 20px; }
  .s40 svg { width: 14px; height: 14px; }
</style>
