<script lang="ts">
  import { tick } from 'svelte';
  import { api, ApiFailure, errorText } from '../lib/api';
  import { safeNext } from '../lib/route';
  import { navigate, session } from '../lib/session.svelte';
  import type { Me } from '../lib/gen/Me';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let errorEl = $state<HTMLElement>();

  async function submit(e: Event) {
    e.preventDefault();
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

<main class="mx-auto max-w-sm space-y-4 p-6">
  <h1 class="text-xl font-medium">Sign in</h1>
  {#if session.notice}<p role="status" class="text-warn">{session.notice}</p>{/if}
  <form onsubmit={submit} class="space-y-3">
    <label class="block">Username
      <input class="w-full rounded bg-element p-2" autocomplete="username" bind:value={username} required />
    </label>
    <label class="block">Password
      <input class="w-full rounded bg-element p-2" type="password" autocomplete="current-password" bind:value={password} required />
    </label>
    <button class="w-full rounded bg-selected p-2" disabled={busy}>Sign in</button>
  </form>
  {#if error}<p role="alert" class="text-danger" tabindex="-1" bind:this={errorEl}>{error}</p>{/if}
</main>
