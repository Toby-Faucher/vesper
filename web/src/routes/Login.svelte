<script lang="ts">
  import { tick } from 'svelte';
  import { api, ApiFailure, errorText } from '../lib/api';
  import { safeNext } from '../lib/route';
  import { navigate, session } from '../lib/session.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Field from '../lib/ui/c/Field.svelte';
  import PasswordInput from '../lib/ui/c/PasswordInput.svelte';
  import TextInput from '../lib/ui/c/TextInput.svelte';
  import AuthCard from './AuthCard.svelte';
  import type { Me } from '../lib/gen/Me';
  import type { ResetNotice } from '../lib/gen/ResetNotice';
  import { UI_MESSAGES as M } from '../lib/ui/messages-first';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let errorEl = $state<HTMLElement>();
  // Resets of this account's password not shown yet: told once, here, before the app opens.
  let notices = $state<ResetNotice[]>([]);
  let continueEl = $state<HTMLButtonElement | null>(null);
  const next = () => navigate(safeNext(new URLSearchParams(location.search).get('next')) ?? '/', true);
  const dayOf = (ms: number) => new Date(ms).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: 'numeric', minute: '2-digit' });

  // Runs one way of signing in (password or passkey); `fail` turns its error into the text shown.
  async function attempt(signIn: () => Promise<Me>, fail: (err: unknown) => string) {
    if (busy) return;
    busy = true;
    error = '';
    try {
      session.state.me = await signIn();
      session.notice = '';
      notices = await api<ResetNotice[]>('POST', '/api/me/reset-notices').catch(() => []);
      if (!notices.length) next();
    } catch (err) {
      error = fail(err);
    } finally {
      password = '';
      busy = false;
    }
    if (error || notices.length) {
      await tick();
      (error ? errorEl : continueEl)?.focus();
    }
  }

  const submit = (e: Event) => {
    e.preventDefault();
    return attempt(
      () => api<Me>('POST', '/api/login', { username, password }),
      (err) => (err instanceof ApiFailure && err.status === 401 ? 'Wrong username or password.' : errorText(err)),
    );
  };
</script>

<AuthCard>
  {#if notices.length}
    <h1 class="mb-3 text-xl font-semibold text-fg">Before you go on</h1>
    <div role="status" class="mb-5 flex flex-col gap-2 text-fg-2">
      {#each notices as n (n.at)}<p>{M.resetNotice(n.by, n.by_username, dayOf(n.at))}</p>{/each}
      <p>If that was not you, tell the owner.</p>
    </div>
    <div class="flex flex-col"><Button variant="primary" bind:el={continueEl} onclick={next}>Continue</Button></div>
  {:else}
  <h1 class="mb-5 text-xl font-semibold text-fg">Sign in</h1>
  {#if session.notice}<p role="status" class="mb-4 text-warn">{session.notice}</p>{/if}
  <form onsubmit={submit} class="flex flex-col gap-4">
    <Field label="Username">
      <TextInput autocomplete="username" autocapitalize="none" spellcheck="false" bind:value={username} required />
    </Field>
    <Field label="Password">
      <PasswordInput autocomplete="current-password" bind:value={password} required />
    </Field>
    <Button variant="primary" type="submit" loading={busy}>Sign in</Button>
  </form>
  {#if window.PublicKeyCredential}
    {#await import('./PasskeyButton.svelte') then m}<m.default {busy} {attempt} />{/await}
  {/if}
  {#if error}<p role="alert" class="mt-4 text-danger" tabindex="-1" bind:this={errorEl}>{error}</p>{/if}
  {/if}
  {#snippet below()}
    <p class="mt-4 text-center text-fg-3">Need an account? Ask for an invite link.</p>
    <p class="mt-1 text-center text-fg-3">Forgot your password? Ask the owner for a reset link.</p>
  {/snippet}
</AuthCard>
