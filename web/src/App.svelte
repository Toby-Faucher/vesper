<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { api, ApiFailure, errorText } from './lib/api';
  import { navigate, route, session } from './lib/session.svelte';
  import Login from './routes/Login.svelte';
  import Signup from './routes/Signup.svelte';
  import Room from './routes/Room.svelte';
  import type { Me } from './lib/gen/Me';

  let error = $state('');

  onMount(() => {
    const pop = () => (route.path = location.pathname);
    window.addEventListener('popstate', pop);
    return () => window.removeEventListener('popstate', pop);
  });

  // `/` needs a session: ask the server who we are, else go to sign in.
  async function load() {
    error = '';
    try {
      session.state.me = await api<Me>('GET', '/api/me');
      session.connect();
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 401) navigate('/login', true);
      else error = errorText(e);
    }
  }
  const home = $derived(route.path !== '/login' && route.path !== '/invite');
  $effect(() => {
    const path = route.path;
    if (!home) return;
    const signedIn = !!session.state.me;
    // untrack: connect() reads the socket state, and a closed socket must not reconnect from here.
    untrack(() => {
      if (path !== '/') navigate('/', true);
      else if (!signedIn) void load();
      else session.connect();
    });
  });
</script>

{#if route.path === '/login'}
  <Login />
{:else if route.path === '/invite'}
  <Signup />
{:else if session.state.me}
  <Room />
{:else if error}
  <main class="mx-auto max-w-sm space-y-3 p-6">
    <p role="alert" class="text-red-400">{error}</p>
    <button class="rounded bg-neutral-700 px-3 py-1" onclick={load}>Try again</button>
  </main>
{/if}
