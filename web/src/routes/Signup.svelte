<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, API_MESSAGES, ApiFailure, errorText } from '../lib/api';
  import { navigate, session, takeInvite } from '../lib/session.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Field from '../lib/ui/c/Field.svelte';
  import PasswordInput from '../lib/ui/c/PasswordInput.svelte';
  import TextInput from '../lib/ui/c/TextInput.svelte';
  import AuthCard from './AuthCard.svelte';
  import type { Me } from '../lib/gen/Me';

  // The code lives in the URL fragment, which the browser never sends to the server. Only the signup body carries it.
  // takeInvite removes it from the address bar and history at once. A used or expired code is only found on submit: the
  // server answers every invite failure the same way, and asking earlier would put the code in a query string.
  const valid = (c: string) => /^[0-9a-f]{64}$/.test(c);
  const first = takeInvite();
  let code = $state(first);
  let badLink = $state(!valid(first));
  let username = $state('');
  let displayName = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let errorEl = $state<HTMLElement>();
  let badEl = $state<HTMLElement>();

  // A second invite link opened in this tab changes only the fragment: take the new code and start over.
  onMount(() => {
    const onhash = () => {
      if (!location.hash) return;
      code = takeInvite();
      badLink = !valid(code);
      error = '';
      password = '';
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
      session.state.me = await api<Me>('POST', '/api/signup', { invite: code, username, password, display_name: displayName });
      navigate('/welcome', true);
    } catch (err) {
      if (err instanceof ApiFailure && err.code === 'invite_invalid') badLink = true;
      else error = errorText(err);
    } finally {
      password = '';
      busy = false;
    }
    await tick();
    (badLink ? badEl : error ? errorEl : undefined)?.focus();
  }
</script>

<AuthCard>
  <h1 class="text-xl font-semibold text-fg">{badLink ? 'You cannot sign up with this link' : 'Create your account'}</h1>
  {#if badLink}
    <p role="alert" class="mt-3 text-fg-2" tabindex="-1" bind:this={badEl}>{API_MESSAGES.invite_invalid}</p>
    <p class="mt-4 text-fg-3">Already have an account? <a class="text-accent-fg underline underline-offset-2" href="/login">Sign in</a></p>
  {:else}
    <p class="mt-1 mb-5 text-fg-2">You were invited to Vesper. Pick a username and a password.</p>
    <form onsubmit={submit} class="flex flex-col gap-4">
      <Field label="Username" help="3 to 24 characters: a to z, 0 to 9 or _. You sign in with it.">
        <TextInput autocomplete="username" autocapitalize="none" spellcheck="false" bind:value={username}
          required minlength={3} maxlength={24} pattern="[A-Za-z0-9_]+" />
      </Field>
      <Field label="Display name" help="What others see. You can change it later.">
        <TextInput autocomplete="nickname" bind:value={displayName} required maxlength={32} />
      </Field>
      {#key code}<!-- a new link remounts it, so the password is hidden again -->
        <Field label="Password" help="8 to 128 characters.">
          <PasswordInput autocomplete="new-password" bind:value={password} required minlength={8} maxlength={128} />
        </Field>
      {/key}
      <div class="mt-2 flex flex-col"><Button variant="primary" type="submit" loading={busy}>Create account</Button></div>
    </form>
    {#if error}<p role="alert" class="mt-4 text-danger" tabindex="-1" bind:this={errorEl}>{error}</p>{/if}
  {/if}
</AuthCard>
