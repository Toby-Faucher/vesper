<script lang="ts">
  // The input level: a bar on a visible track from -100 to 0 dBFS, the number beside it, and an optional marker line.
  // The owner sets `db` at most 10 times a second; the bar moves by transform, with no transition (nothing to reduce).
  let { db, marker, label = 'Input level' }: { db: number; marker?: number; label?: string } = $props();
  const v = $derived(Math.round(Math.max(-100, Math.min(0, db))));
  const text = $derived(v <= -100 ? 'Silent' : `${v} dBFS`);
</script>

<div class="flex items-center gap-3">
  <div class="track" role="meter" aria-label={label} aria-valuemin={-100} aria-valuemax={0} aria-valuenow={v} aria-valuetext={text}>
    <div class="bar" class:hot={v >= -3} style:transform="scaleX({(v + 100) / 100})"></div>
    {#if marker !== undefined}<div class="mark" style:left="{marker + 100}%"></div>{/if}
  </div>
  <span class="w-16 shrink-0 text-right text-xs text-fg-2 tabular-nums" aria-hidden="true">{text}</span>
</div>

<style>
  .track {
    position: relative;
    flex: 1;
    height: 8px;
    overflow: hidden;
    border-radius: 4px;
    background: var(--bg-selected);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .bar { height: 100%; background: var(--ok-fg); transform-origin: left; }
  .hot { background: var(--warn-fg); }
  .mark { position: absolute; top: 0; width: 2px; height: 100%; margin-left: -1px; background: var(--fg); }
</style>
