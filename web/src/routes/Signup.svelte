<script lang="ts">
  import { tick } from 'svelte';
  import { api, ApiFailure, errorText } from '../lib/api';
  import { navigate, session, takeInvite } from '../lib/session.svelte';
  import type { Me } from '../lib/gen/Me';

  // The code lives in the URL fragment, which the browser never sends to the server. Only the signup body carries it.
  // takeInvite removes it from the address bar and history at once.
  const code = takeInvite();
  const BAD_LINK = 'This invite link is not valid. Ask for a new one.';
  let badLink = $state(!/^[0-9a-f]{64}$/.test(code));
  let username = $state('');
  let displayName = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let errorEl = $state<HTMLElement>();

  async function submit(e: Event) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      session.state.me = await api<Me>('POST', '/api/signup', { invite: code, username, password, display_name: displayName });
      navigate('/', true);
    } catch (err) {
      if (err instanceof ApiFailure && err.code === 'invite_invalid') badLink = true;
      else error = errorText(err);
    } finally {
      password = '';
      busy = false;
    }
    if (error) {
      await tick();
      errorEl?.focus();
    }
  }
</script>

<main class="mx-auto max-w-sm space-y-4 p-6">
  <h1 class="text-xl font-semibold">Create your account</h1>
  {#if badLink}
    <p role="alert" class="text-red-400">{BAD_LINK}</p>
  {:else}
    <form onsubmit={submit} class="space-y-3">
      <label class="block">Username
        <input class="w-full rounded bg-neutral-900 p-2" autocomplete="username" aria-describedby="username-help"
          bind:value={username} required minlength="3" maxlength="24" pattern="[A-Za-z0-9_]+" />
      </label>
      <p id="username-help" class="text-sm text-neutral-400">3 to 24 letters, digits or _. You sign in with it.</p>
      <label class="block">Display name
        <input class="w-full rounded bg-neutral-900 p-2" autocomplete="nickname" bind:value={displayName} required maxlength="32" />
      </label>
      <label class="block">Password
        <input class="w-full rounded bg-neutral-900 p-2" type="password" autocomplete="new-password" aria-describedby="password-help"
          bind:value={password} required minlength="8" maxlength="128" />
      </label>
      <p id="password-help" class="text-sm text-neutral-400">At least 8 characters.</p>
      <button class="w-full rounded bg-neutral-700 p-2" disabled={busy}>Create account</button>
    </form>
    {#if error}<p role="alert" class="text-red-400" tabindex="-1" bind:this={errorEl}>{error}</p>{/if}
  {/if}
</main>
