<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, errorText } from '../lib/api';
  import { navigate, session } from '../lib/session.svelte';
  import { updateSettings, voice } from '../lib/voice.svelte';
  import { SLOT_HEX, avatarSlot } from '../lib/ui/avatar';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Field from '../lib/ui/c/Field.svelte';
  import TextInput from '../lib/ui/c/TextInput.svelte';
  import AuthCard from './AuthCard.svelte';
  import MicCheck from './settings/MicCheck.svelte';
  import type { UserInfo } from '../lib/gen/UserInfo';

  // First run, after signup (and again from Settings > Account): name and colour, the mic, mentions. Every step can be
  // skipped (the button or Esc). Finishing or skipping marks this account as welcomed on this browser.
  const COLOURS = ['Coral', 'Orange', 'Lime', 'Green', 'Teal', 'Sky', 'Blue', 'Pink'];
  const me = $derived(session.state.me!.user);
  let step = $state(1);
  let name = $state(session.state.me!.user.display_name);
  let slot = $state(avatarSlot(session.state.me!.user)); // 1 to 8
  let saving = $state(false);
  let saveError = $state('');
  let heading = $state<HTMLElement>();
  const supported = typeof Notification !== 'undefined';
  // Read, never asked, on load: the browser asks only from the button below.
  let permission = $state<NotificationPermission | 'unsupported'>(supported ? Notification.permission : 'unsupported');
  const PERMISSION_TEXT: Record<NotificationPermission | 'unsupported', string> = {
    default: '',
    granted: 'Notifications are on. You get one when someone mentions you.',
    denied: "Notifications are blocked for this site. You can allow them in the browser's site settings.",
    unsupported: 'This browser does not show notifications.',
  };

  async function go(to: number) {
    step = to;
    await tick();
    heading?.focus();
  }
  function finish() {
    try { localStorage.setItem(`vesper.welcomed.${me.user_id}`, '1'); } catch { /* storage blocked */ }
    navigate('/', true); // `/` opens the last room, else the first
  }
  async function saveProfile(e: Event) {
    e.preventDefault();
    if (saving) return;
    const patch: { display_name?: string; accent?: string } = {};
    if (name.trim() !== me.display_name) patch.display_name = name;
    // Against the stored value: the server default is no slot, so the colour shown (from the id hash) is saved too.
    if (SLOT_HEX[slot - 1] !== me.accent.toLowerCase()) patch.accent = SLOT_HEX[slot - 1];
    saveError = '';
    if (Object.keys(patch).length) {
      saving = true;
      try {
        const user = await api<UserInfo>('PATCH', '/api/me', patch);
        session.state.me = { ...session.state.me!, user };
      } catch (err) {
        saveError = errorText(err);
      } finally {
        saving = false;
      }
      if (saveError) return;
    }
    await go(2);
  }
  async function askNotifications() {
    // shortcut: a plain permission request; Task 14's requestMentions() takes this over.
    permission = await Notification.requestPermission();
  }

  onMount(() => {
    heading?.focus();
    const onkey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) finish();
    };
    window.addEventListener('keydown', onkey);
    return () => window.removeEventListener('keydown', onkey);
  });
</script>

<AuthCard wide>
  {#snippet aside()}<span class="text-fg-3">Step {step} of 3</span>{/snippet}

  {#if step === 1}
    <form onsubmit={saveProfile} class="flex flex-col gap-5">
      <div>
        <h1 bind:this={heading} tabindex="-1" class="text-xl font-semibold text-fg outline-none">How others see you</h1>
        <p class="mt-1 text-fg-2">Your name and colour show next to your messages and in voice.</p>
      </div>
      <div class="flex items-center gap-3 rounded-md bg-app p-3" aria-hidden="true">
        <Avatar user={{ user_id: me.user_id, display_name: name || me.username, accent: SLOT_HEX[slot - 1] }} size={40} />
        <span class="min-w-0 truncate text-base font-medium text-fg">{name || me.username}</span>
      </div>
      <Field label="Display name" help="1 to 32 characters." error={saveError || undefined}>
        <TextInput autocomplete="nickname" bind:value={name} required maxlength={32} />
      </Field>
      <fieldset>
        <legend class="mb-2 text-sm font-medium text-fg">Colour</legend>
        <div class="flex gap-2.5">
          {#each SLOT_HEX as _, i (i)}
            <label class="swatch" style:--c="var(--avatar-{i + 1})">
              <input type="radio" name="colour" class="absolute inset-0 m-0 cursor-pointer appearance-none rounded-full outline-none" aria-label={COLOURS[i]} value={i + 1} bind:group={slot} />
            </label>
          {/each}
        </div>
      </fieldset>
      <div class="flex items-center gap-2 pt-1">
        <Button variant="ghost" onclick={finish}>Skip for now</Button>
        <span class="ml-auto"></span>
        <Button variant="primary" type="submit" loading={saving}>Next</Button>
      </div>
    </form>
  {:else if step === 2}
    <div class="flex flex-col gap-4">
      <div>
        <h1 bind:this={heading} tabindex="-1" class="text-xl font-semibold text-fg outline-none">Check your microphone</h1>
        <p class="mt-1 text-fg-2">Speak. The bar should move. If it does not, pick another microphone.</p>
      </div>
      <MicCheck settings={voice.settings} onchange={updateSettings} marker={false} />
      <div class="flex items-center gap-2 pt-1">
        <Button variant="ghost" onclick={finish}>Skip for now</Button>
        <span class="ml-auto"></span>
        <Button onclick={() => go(1)}>Back</Button>
        <Button variant="primary" onclick={() => go(3)}>Next</Button>
      </div>
    </div>
  {:else}
    <div class="flex flex-col gap-4">
      <div>
        <h1 bind:this={heading} tabindex="-1" class="text-xl font-semibold text-fg outline-none">Mentions</h1>
        <p class="mt-1 text-fg-2">Get a browser notification when someone mentions you. Vesper does not notify you for anything else.</p>
      </div>
      {#if permission === 'default'}
        <div><Button onclick={askNotifications}>Turn on notifications</Button></div>
      {/if}
      <p role="status" class="text-fg-2" class:hidden={permission === 'default'}>{PERMISSION_TEXT[permission]}</p>
      <div class="flex items-center gap-2 pt-1">
        <Button variant="ghost" onclick={finish}>Skip for now</Button>
        <span class="ml-auto"></span>
        <Button onclick={() => go(2)}>Back</Button>
        <Button variant="primary" onclick={finish}>Open Vesper</Button>
      </div>
    </div>
  {/if}
</AuthCard>

<style>
  .swatch {
    position: relative;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--c);
    cursor: pointer;
    box-shadow: 0 0 0 2px var(--bg-raised), 0 0 0 4px transparent;
    transition: box-shadow var(--dur-1) var(--ease-out);
  }
  .swatch:has(:checked) { box-shadow: 0 0 0 2px var(--bg-raised), 0 0 0 4px var(--fg); }
  .swatch:has(:focus-visible) { outline: 2px solid var(--focus); outline-offset: 5px; }
</style>
