<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { shares } from '../lib/share/state.svelte';
  import { engineOf, ceilingKbps, suggestKbps, MIN_KBPS } from '../lib/share/ceiling';
  import { loadShareSettings, saveShareSettings, type Prefer } from '../lib/share/prefs';
  import { measureUploadKbps } from '../lib/share/speedtest';
  import { startShare, updateShare } from '../lib/voice.svelte';
  import { UI_MESSAGES as M } from '../lib/ui/messages';
  import Button from '../lib/ui/c/Button.svelte';
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import Field from '../lib/ui/c/Field.svelte';
  import Range from '../lib/ui/c/Range.svelte';
  import Select from '../lib/ui/c/Select.svelte';
  // Settings for a share: before it starts (the picture size is the screen's, the upload is measured once) or while it runs
  // (`edit`: the size is the captured track's, nothing is measured, so the test does not eat the share's own bandwidth).
  let { edit = false, onclose }: { edit?: boolean; onclose: () => void } = $props();

  const editing = untrack(() => edit); // the mode does not change while the dialog is open
  const saved = untrack(loadShareSettings);
  const engine = engineOf(navigator.userAgent);
  const size = untrack(() => {
    const t = shares.mine?.track.getSettings();
    const r = window.devicePixelRatio || 1;
    return edit && t?.width ? { w: t.width, h: t.height ?? 0 } : { w: screen.width * r, h: screen.height * r };
  });
  const ceiling = ceilingKbps(engine, size.w, size.h);
  const mbps = (kbps: number) => +(kbps / 1000).toFixed(1);

  let fps = $state<30 | 60>(untrack(() => (edit && shares.mine ? shares.mine.fps : saved.fps)));
  let prefer = $state<Prefer>(untrack(() => (edit && shares.mine ? shares.mine.prefer : saved.prefer)));
  let kbps = $state(Math.min(ceiling, untrack(() => (edit && shares.mine ? shares.mine.kbps : saved.kbps ?? suggestKbps(null, ceiling)))));
  let upload = $state<number | null | undefined>(editing ? null : undefined); // undefined: measuring
  let moved = false; // the person moved the slider: a later measurement does not overwrite it
  let closed = false;
  onDestroy(() => (closed = true));
  if (!editing) void measureUploadKbps().then((u) => {
    if (closed) return;
    upload = u;
    if (saved.kbps === null && u !== null && !moved) kbps = suggestKbps(u, ceiling);
  });

  function go() {
    const s = { fps, kbps, prefer };
    saveShareSettings(s);
    onclose();
    void (editing ? updateShare(s) : startShare(s));
  }
</script>

<Dialog open title={editing ? M.shareEditTitle : M.shareTitle} {onclose}>
  <div class="flex flex-col gap-4">
    <Field label={M.shareFps}>
      <Select bind:value={fps}>
        <option value={30}>30 fps</option>
        <option value={60}>60 fps</option>
      </Select>
    </Field>
    <Field label={M.shareBitrate} value={M.shareMbps(mbps(kbps))}>
      <Range bind:value={kbps} oninput={() => (moved = true)} min={MIN_KBPS} max={ceiling} step={500} aria-valuetext={M.shareMbps(mbps(kbps))} />
    </Field>
    {#if !editing}
      <p class="text-xs text-fg-2" role="status">
        {#if upload === undefined}{M.shareMeasuring}
        {:else if upload === null}{M.shareMeasureFailed}
        {:else}{M.shareMeasured(mbps(upload), mbps(suggestKbps(upload, ceiling)))}{/if}
      </p>
    {/if}
    <a class="text-xs text-fg-2 underline underline-offset-2 hover:text-fg" href="https://speed.cloudflare.com" target="_blank" rel="noopener noreferrer">{M.shareSpeed}</a>
    {#if engine === 'firefox'}<p class="text-xs text-fg-2">{M.shareFirefox(mbps(ceiling))}</p>{/if}
    <Field label={M.shareStruggle}>
      <Select bind:value={prefer}>
        <option value="sharp">{M.sharePreferSharp}</option>
        <option value="smooth">{M.sharePreferSmooth}</option>
      </Select>
    </Field>
    <div class="flex justify-end gap-2">
      <Button variant="ghost" onclick={onclose}>{M.shareCancel}</Button>
      <Button variant="primary" onclick={go}>{editing ? M.shareSave : M.shareStart}</Button>
    </div>
    <p class="text-xs text-fg-3">{M.shareNothing}</p>
  </div>
</Dialog>
