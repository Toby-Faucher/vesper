<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { api, ApiFailure, errorText } from './lib/api';
  import { loginHref, parse } from './lib/route';
  import { navigate, route, session } from './lib/session.svelte';
  import { applyAccent } from './lib/ui/accent';
  import Login from './routes/Login.svelte';
  import Shell from './routes/Shell.svelte';
  import type { Me } from './lib/gen/Me';
  import type { Component } from 'svelte';

  let error = $state('');
  // The token gallery at /_gallery, in the dev server only: the build drops this import.
  let Gallery = $state<Component | null>(null);
  if (import.meta.env.DEV) void import('./routes/dev/Gallery.svelte').then((m) => (Gallery = m.default));

  onMount(() => {
    const pop = () => (route.path = location.pathname);
    window.addEventListener('popstate', pop);
    return () => window.removeEventListener('popstate', pop);
  });

  // Every page but sign in and invite needs a session: ask the server who we are, else go to sign in.
  async function load() {
    error = '';
    try {
      session.state.me = await api<Me>('GET', '/api/me');
      session.connect();
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 401) navigate(loginHref(location.pathname), true); // sign in returns here, so a shared room link survives it
      else error = errorText(e);
    }
  }
  // The accent follows the account: painted before the shell renders, and again when another device changes it.
  $effect.pre(() => {
    const me = session.state.me;
    if (me) applyAccent(me.user.ui_accent);
  });
  const r = $derived(parse(route.path));
  const gallery = $derived(import.meta.env.DEV && r.name === 'gallery');
  const inShell = $derived(r.name !== 'login' && r.name !== 'invite' && !gallery);
  $effect(() => {
    if (!inShell) return;
    const signedIn = !!session.state.me;
    // untrack: connect() reads the socket state, and a closed socket must not reconnect from here.
    untrack(() => (signedIn ? session.connect() : void load()));
  });
</script>

{#if gallery}
  {#if Gallery}<Gallery />{/if}
{:else if r.name === 'login'}
  <Login />
{:else if r.name === 'invite'}
  {#await import('./routes/Signup.svelte') then m}<m.default />{:catch}{@render failed()}{/await}
{:else if session.state.me}
  <Shell />
{:else if error}
  <main class="mx-auto max-w-sm space-y-3 p-6">
    <p role="alert" class="text-danger">{error}</p>
    <button class="rounded bg-selected px-3 py-1" onclick={load}>Try again</button>
  </main>
{/if}

{#snippet failed()}
  <main class="mx-auto max-w-sm p-6"><p role="alert" class="text-danger">Could not load this page. Check your connection and reload.</p></main>
{/snippet}
