<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, errorText } from '../lib/api';
  import { navigate, session } from '../lib/session.svelte';
  import { updateSettings, voice } from '../lib/voice.svelte';
  import { loadPrefs, permission, requestMentions } from '../lib/ui/notify';
  import { SLOT_HEX, avatarSlot, profilePatch } from '../lib/ui/avatar';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Field from '../lib/ui/c/Field.svelte';
  import TextInput from '../lib/ui/c/TextInput.svelte';
  import AuthCard from './AuthCard.svelte';
  import Swatches from './Swatches.svelte';
  import MicCheck from './settings/MicCheck.svelte';
  import type { UserInfo } from '../lib/gen/UserInfo';

  // First run, after signup (and again from Settings > Account): name and colour, the mic, mentions. Every step can be
  // skipped with its button. No Esc: a stray Esc in the name field must not throw away what was typed. It renders in the
  // Shell's pane, so a call keeps running while it is open.
  const me = $derived(session.state.me!.user);
  let step = $state(1);
  let name = $state(session.state.me!.user.display_name);
  let slot = $state(avatarSlot(session.state.me!.user)); // 1 to 8
  let saving = $state(false);
  let saveError = $state('');
  let heading = $state<HTMLElement>();
  // Read, never asked, on load: the browser asks only from the button below. `granted` here also means mentions are on.
  const current = () => (permission() === 'granted' && !loadPrefs().mentions ? 'default' : permission());
  let perm = $state(current());
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
  const finish = () => navigate('/', true); // `/` opens the last room, else the first
  async function saveProfile(e: Event) {
    e.preventDefault();
    if (saving) return;
    const patch = profilePatch(me, name, slot);
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
    await requestMentions(); // turns mentions on when allowed
    perm = current();
  }

  onMount(() => heading?.focus());
</script>

<AuthCard wide pane>
  {#snippet aside()}<span class="text-fg-3">Step {step} of 3</span>{/snippet}

  {#if step === 1}
    <form onsubmit={saveProfile} class="flex flex-col gap-5">
      <div>
        <h1 bind:this={heading} tabindex="-1" class="text-xl font-semibold text-fg outline-none">How others see you</h1>
        <p class="mt-2 text-fg-2">Your name and colour show next to your messages and in voice.</p>
      </div>
      <div class="flex items-center gap-3 rounded-md bg-app p-3" aria-hidden="true">
        <Avatar user={{ user_id: me.user_id, display_name: name || me.username, accent: SLOT_HEX[slot - 1] }} size={40} />
        <span class="min-w-0 truncate text-base font-medium text-fg">{name || me.username}</span>
      </div>
      <Field label="Display name" help="1 to 32 characters." error={saveError || undefined}>
        <TextInput autocomplete="nickname" bind:value={name} required maxlength={32} />
      </Field>
      <Swatches bind:slot />
      <div class="flex items-center gap-2 pt-1">
        <span class="-ml-3"><Button variant="ghost" onclick={finish}>Skip for now</Button></span>
        <span class="ml-auto"></span>
        <Button variant="primary" type="submit" loading={saving}>Next</Button>
      </div>
    </form>
  {:else if step === 2}
    <div class="flex flex-col gap-4">
      <div>
        <h1 bind:this={heading} tabindex="-1" class="text-xl font-semibold text-fg outline-none">Check your microphone</h1>
        <p class="mt-2 text-fg-2">Speak. The bar should move. If it does not, pick another microphone.</p>
      </div>
      <MicCheck settings={voice.settings} onchange={updateSettings} marker={false} />
      <div class="flex items-center gap-2 pt-1">
        <span class="-ml-3"><Button variant="ghost" onclick={finish}>Skip for now</Button></span>
        <span class="ml-auto"></span>
        <Button onclick={() => go(1)}>Back</Button>
        <Button variant="primary" onclick={() => go(3)}>Next</Button>
      </div>
    </div>
  {:else}
    <div class="flex flex-col gap-4">
      <div>
        <h1 bind:this={heading} tabindex="-1" class="text-xl font-semibold text-fg outline-none">Mentions</h1>
        <p class="mt-2 text-fg-2">Get a browser notification when someone mentions you. Vesper does not notify you for anything else.</p>
      </div>
      <!-- The live region stays rendered (empty while not asked), so its first text is announced. -->
      <div>
        {#if perm === 'default'}<Button onclick={askNotifications}>Turn on notifications</Button>{/if}
        <p role="status" class="text-fg-2">{PERMISSION_TEXT[perm]}</p>
      </div>
      <div class="flex items-center gap-2 pt-1">
        <span class="-ml-3"><Button variant="ghost" onclick={finish}>Skip for now</Button></span>
        <span class="ml-auto"></span>
        <Button onclick={() => go(2)}>Back</Button>
        <Button variant="primary" onclick={finish}>Open Vesper</Button>
      </div>
    </div>
  {/if}
</AuthCard>

