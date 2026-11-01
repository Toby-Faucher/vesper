<script lang="ts">
  import { STYLES, STYLE_KEY, applyStyle, loadStyle, type StyleId } from '../../lib/ui/style';
  // Settings > Appearance: the style, saved to vesper.style and applied at once. One style for now.
  const store = (() => { try { return localStorage; } catch { return null; } })();
  let style = $state<StyleId>(loadStyle(store));
  function pick(id: StyleId) {
    style = id;
    try { store?.setItem(STYLE_KEY, id); } catch { /* storage blocked: applied for this visit only */ }
    applyStyle(id);
  }
</script>

<div class="flex flex-col gap-4">
  <fieldset class="flex flex-col gap-2" aria-describedby="style-later">
    <legend class="mb-2 text-base font-medium text-fg">Style</legend>
    {#each STYLES as s (s.id)}
      <label class="option flex h-10 cursor-pointer items-center gap-3 rounded-md px-3 text-sm text-fg">
        <input type="radio" name="style" value={s.id} checked={style === s.id} onchange={() => pick(s.id)} />
        {s.name}
      </label>
    {/each}
    <p id="style-later" class="mt-1 text-sm text-fg-2">More styles will come later.</p>
  </fieldset>
  <p class="text-sm text-fg-2">When your system asks for less motion, Vesper fades panels in place of sliding them.</p>
</div>

<style>
  .option { max-width: 320px; border: 1px solid var(--line); }
  .option:hover { background: var(--bg-hover); }
  .option:has(:checked) { border-color: var(--line-strong); background: var(--bg-selected); }
  .option:has(:focus-visible) { outline: 2px solid var(--focus); outline-offset: 2px; }
  input { margin: 0; accent-color: var(--accent); outline: none; }
</style>
