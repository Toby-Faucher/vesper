<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, API_MESSAGES, ApiFailure, errorText } from '../lib/api';
  import { navigate, takeInvite } from '../lib/session.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Field from '../lib/ui/c/Field.svelte';
  import PasswordInput from '../lib/ui/c/PasswordInput.svelte';
  import AuthCard from './AuthCard.svelte';
  import type { ResetReq } from '../lib/gen/ResetReq';

  // /reset#code, made by the owner or a moderator (Members) or `vesper-server reset-password`. As on the invite page, the
  // code stays in the fragment, which the browser never sends, and leaves the address bar at once. A used or old code
  // is only found on submit: the server answers every bad code the same way. Success does not sign in: every session of
  // the account just ended, and the person signs in with the new password next.
  const valid = (c: string) => /^[0-9a-f]{64}$/.test(c);
  const first = takeInvite();
  let code = $state(first);
  let badLink = $state(!valid(first));
  let done = $state(false);
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let errorEl = $state<HTMLElement>();
  let focusEl = $state<HTMLElement>();

  onMount(() => {
    const onhash = () => {
      if (!location.hash) return;
      code = takeInvite();
      [badLink, done, error, password] = [!valid(code), false, '', ''];
    };
    window.addEventListener('hashchange', onhash);
    return () => window.removeEventListener('hashchange', onhash);
  });

  async function submit(e: Event) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    error = '';
    try {
      const body: ResetReq = { code, password };
      await api('POST', '/api/reset', body);
      done = true;
    } catch (err) {
      if (err instanceof ApiFailure && err.code === 'reset_invalid') badLink = true;
      else error = errorText(err);
    } finally {
      password = '';
      busy = false;
    }
    await tick();
    (badLink || done ? focusEl : error ? errorEl : undefined)?.focus();
  }
</script>

<AuthCard>
  {#if done}
    <h1 class="text-xl font-semibold text-fg" tabindex="-1" bind:this={focusEl}>Your password is set</h1>
    <p role="status" class="mt-3 text-fg-2">You are signed out on every device. Sign in with your new password.</p>
    <div class="mt-5 flex flex-col"><Button variant="primary" onclick={() => navigate('/login', true)}>Sign in</Button></div>
  {:else if badLink}
    <h1 class="text-xl font-semibold text-fg">You cannot reset your password with this link</h1>
    <p role="alert" class="mt-3 text-fg-2" tabindex="-1" bind:this={focusEl}>{API_MESSAGES.reset_invalid}</p>
    <p class="mt-4 text-fg-3">Remember it after all? <a class="text-accent-fg underline underline-offset-2" href="/login">Sign in</a></p>
  {:else}
    <h1 class="text-xl font-semibold text-fg">Set a new password</h1>
    <p class="mt-1 mb-5 text-fg-2">This link works once. Using it signs you out on every device.</p>
    <form onsubmit={submit} class="flex flex-col gap-4">
      {#key code}<!-- a new link remounts it, so the password is hidden again -->
        <Field label="New password" help="8 to 128 characters.">
          <PasswordInput autocomplete="new-password" bind:value={password} required minlength={8} maxlength={128} />
        </Field>
      {/key}
      <div class="mt-2 flex flex-col"><Button variant="primary" type="submit" loading={busy}>Set password</Button></div>
    </form>
    {#if error}<p role="alert" class="mt-4 text-danger" tabindex="-1" bind:this={errorEl}>{error}</p>{/if}
  {/if}
</AuthCard>
