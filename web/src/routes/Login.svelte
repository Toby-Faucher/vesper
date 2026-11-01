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

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let errorEl = $state<HTMLElement>();

  async function submit(e: Event) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    error = '';
    try {
      session.state.me = await api<Me>('POST', '/api/login', { username, password });
      session.notice = '';
      navigate(safeNext(new URLSearchParams(location.search).get('next')) ?? '/', true);
    } catch (err) {
      error = err instanceof ApiFailure && err.status === 401 ? 'Wrong username or password.' : errorText(err);
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

<AuthCard>
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
  {#if error}<p role="alert" class="mt-4 text-danger" tabindex="-1" bind:this={errorEl}>{error}</p>{/if}
  {#snippet below()}
    <p class="mt-4 text-center text-fg-3">Need an account? Ask for an invite link.</p>
  {/snippet}
</AuthCard>
