<script lang="ts">
  import { api, errorText } from '../../lib/api';
  import { session } from '../../lib/session.svelte';
  import { EXAMPLE, PRESETS, SURFACE, applyAccent, check, contrast, fillOf, isHex, nearest } from '../../lib/ui/accent';
  import * as I from '../../lib/ui/icons';
  import { UI_MESSAGES as M } from '../../lib/ui/messages';
  import { STYLES, STYLE_KEY, applyStyle, loadStyle, type StyleId } from '../../lib/ui/style';
  import Button from '../../lib/ui/c/Button.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import Icon from '../../lib/ui/c/Icon.svelte';
  import Status from '../../lib/ui/c/Status.svelte';
  import TextInput from '../../lib/ui/c/TextInput.svelte';
  import type { UserInfo } from '../../lib/gen/UserInfo';
  // Settings > Appearance: the style (this browser, vesper.style) and the accent (the account, ui_accent on the server).
  const store = (() => { try { return localStorage; } catch { return null; } })();
  let style = $state<StyleId>(loadStyle(store));
  function pick(id: StyleId) {
    style = id;
    try { store?.setItem(STYLE_KEY, id); } catch { /* storage blocked: applied for this visit only */ }
    applyStyle(id);
  }

  // A preset key or #rrggbb. It paints at once (the preview), then saves; a failed save puts the saved one back.
  const start = session.state.me!.user.ui_accent ?? 'amber';
  let accent = $state(start);
  let custom = $state(isHex(start) ? start : '');
  let error = $state('');
  let seq = 0; // arrow keys through the swatches send one save each: only the last answer counts
  async function save(v: string) {
    const n = ++seq;
    [accent, error] = [v, ''];
    applyAccent(v);
    try {
      const user = await api<UserInfo>('PATCH', '/api/me', { ui_accent: v });
      if (n !== seq) return;
      session.state.me = { ...session.state.me!, user };
      session.state.users = { ...session.state.users, [user.user_id]: user };
    } catch (e) {
      if (n !== seq) return;
      error = errorText(e);
      accent = session.state.me!.user.ui_accent ?? 'amber';
      applyAccent(accent);
    }
  }
  const hex = $derived(custom.trim().toLowerCase());
  const verdict = $derived(isHex(hex) ? check(hex) : null);
  const ratio = (r: number) => Math.floor(r * 10) / 10; // never rounds a 4.47 up to the 4.5 it needs
</script>

<div class="flex flex-col gap-8">
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

  <fieldset class="flex max-w-120 flex-col gap-3" aria-describedby="accent-help">
    <legend class="mb-1 text-base font-medium text-fg">Accent</legend>
    <p id="accent-help" class="text-sm text-fg-2">Buttons, focus, selection and mentions use it, on every device you sign in on.</p>
    <div class="grid grid-cols-4 gap-2">
      {#each Object.entries(PRESETS) as [key, p] (key)}
        <label class="option relative flex h-10 cursor-pointer items-center gap-2 rounded-md px-2.5 text-sm text-fg">
          <input type="radio" name="accent" class="absolute inset-0 cursor-pointer appearance-none rounded-md" value={key} checked={accent === key} onchange={() => save(key)} />
          <span class="dot" style:--c={p.hex}></span>
          <span class="flex-1">{p.name}</span>
          {#if accent === key}<Icon d={I.check} />{/if}
        </label>
      {/each}
    </div>
    <div class="flex flex-col gap-2">
      <Field label="Custom colour" help={!hex ? undefined : verdict ? undefined : M.accentFormat(EXAMPLE)}>
        <div class="flex gap-2">
          <input type="color" class="pick" aria-label="Pick a custom colour" value={verdict ? hex : fillOf(accent)} oninput={(e) => (custom = e.currentTarget.value)} />
          <div class="w-28"><TextInput bind:value={custom} placeholder={EXAMPLE} maxlength={7} spellcheck="false" autocomplete="off" /></div>
          <Button disabled={!verdict?.ok || accent === hex} onclick={() => save(hex)}>Use colour</Button>
        </div>
      </Field>
      {#if verdict?.ok}
        <Status kind="info">{accent === hex ? M.accentInUse : M.accentOk(ratio(contrast(SURFACE.app, hex)))}</Status>
      {:else if verdict}
        <Status kind="warn">
          <p>{M.accentRefused(verdict.what, ratio(verdict.ratio), verdict.need)}</p>
          <Button size="sm" variant="ghost" onclick={() => (custom = nearest(hex))}>Use nearest allowed: {nearest(hex)}</Button>
        </Status>
      {/if}
    </div>
    {#if error}<Status kind="error">{error}</Status>{/if}
    <div><Button disabled={accent === 'amber'} onclick={() => save('amber')}>Reset to amber</Button></div>
  </fieldset>

  <p class="text-sm text-fg-2">When your system asks for less motion, Vesper fades panels in place of sliding them.</p>
</div>

<style>
  .option { max-width: 320px; border: 1px solid var(--line); }
  .option:hover { background: var(--bg-hover); }
  .option:has(:checked) { border-color: var(--line-strong); background: var(--bg-selected); }
  .option:has(:focus-visible) { outline: 2px solid var(--focus); outline-offset: 2px; }
  input[type='radio'] { margin: 0; accent-color: var(--accent); outline: none; }
  .dot { width: 16px; height: 16px; flex: none; border-radius: 50%; background: var(--c); }
  .pick { width: 32px; height: 32px; padding: 0; border: 1px solid var(--line); border-radius: var(--radius); background: var(--bg-element); cursor: pointer; }
  .pick:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
</style>
