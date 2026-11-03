<script lang="ts">
  import { onMount } from 'svelte';
  import { ApiFailure, api, errorText } from '../../lib/api';
  import type { PasskeyInfo } from '../../lib/gen/PasskeyInfo';
  import type { PasskeyList } from '../../lib/gen/PasskeyList';
  import { passkeyFailure } from './passkeys';
  import { relTime } from '../../lib/ui/messages';
  import Button from '../../lib/ui/c/Button.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import TextInput from '../../lib/ui/c/TextInput.svelte';
  // Settings > Account > Passkeys: add, name and remove the passkeys that can sign in to this account. A password still works.
  const supported = typeof window !== 'undefined' && !!window.PublicKeyCredential;
  let list = $state<PasskeyList | null>(null);
  let failed = $state(false);
  let name = $state('');
  let adding = $state(false);
  let error = $state('');
  let renaming = $state<number | null>(null);
  let renameTo = $state('');
  let now = $state(Date.now());

  async function refresh() {
    try {
      [list, now, failed] = [await api<PasskeyList>('GET', '/api/passkeys'), Date.now(), false];
    } catch {
      failed = true;
    }
  }

  async function add(e: Event) {
    e.preventDefault();
    if (adding) return;
    [adding, error] = [true, ''];
    try {
      const { addPasskey } = await import('../../lib/passkey');
      await addPasskey(name);
      name = '';
      await refresh();
    } catch (err) {
      error = passkeyFailure(err);
    } finally {
      adding = false;
    }
  }

  async function rename(p: PasskeyInfo) {
    error = '';
    try {
      await api('PATCH', `/api/passkeys/${p.id}`, { name: renameTo });
      renaming = null;
      await refresh();
    } catch (err) {
      error = err instanceof ApiFailure && err.status === 400 ? 'Use 1 to 32 characters for the name.' : errorText(err);
    }
  }

  async function remove(p: PasskeyInfo) {
    error = '';
    try {
      await api('DELETE', `/api/passkeys/${p.id}`);
      await refresh();
    } catch (err) {
      error = errorText(err);
    }
  }

  onMount(() => void refresh());
</script>

<section role="group" aria-labelledby="account-passkeys">
  <div>
    <h3 id="account-passkeys" class="text-base font-medium text-fg">Passkeys</h3>
    <p class="mt-1 text-sm text-fg-2">
      Sign in with your fingerprint, face, screen lock or a security key instead of typing a password. Your password keeps working
      and is how you get back in if you lose a passkey.
    </p>
  </div>
  {#if failed}
    <p role="alert" class="text-sm text-fg-2">Your passkeys could not be loaded. Reload the page to try again.</p>
  {:else if !list}
    <p class="text-sm text-fg-2">Loading your passkeys.</p>
  {:else}
    {#if list.passkeys.length}
      <ul aria-label="Your passkeys" class="keys">
        {#each list.passkeys as p (p.id)}
          <li class="flex items-center gap-4 px-3 py-2.5">
            {#if renaming === p.id}
              <form class="flex min-w-0 flex-1 items-end gap-2" onsubmit={(e) => { e.preventDefault(); void rename(p); }}>
                <Field label="Passkey name"><TextInput bind:value={renameTo} maxlength={32} required /></Field>
                <Button size="sm" type="submit" variant="primary">Save</Button>
                <Button size="sm" variant="ghost" onclick={() => (renaming = null)}>Cancel</Button>
              </form>
            {:else}
              <div class="min-w-0 flex-1" id="passkey-{p.id}">
                <p class="truncate text-sm font-medium text-fg">{p.name}</p>
                <p class="text-xs text-fg-2">
                  Added {new Date(p.created_at).toLocaleDateString()}.
                  {p.last_used_at ? `Last used ${relTime(p.last_used_at, now)}.` : 'Not used yet.'}
                </p>
              </div>
              <Button size="sm" variant="ghost" aria-describedby="passkey-{p.id}" onclick={() => ([renaming, renameTo] = [p.id, p.name])}>Rename</Button>
              <Button size="sm" aria-describedby="passkey-{p.id}" onclick={() => remove(p)}>Remove</Button>
            {/if}
          </li>
        {/each}
      </ul>
    {:else}
      <p class="text-sm text-fg-2">No passkeys yet.</p>
    {/if}
    {#if !list.enabled}
      <p class="text-sm text-fg-2">Passkeys are off on this server: its address is an IP number, and passkeys need a domain name.</p>
    {:else if !supported}
      <p class="text-sm text-fg-2">This browser cannot make passkeys.</p>
    {:else}
      <form class="flex items-end gap-2" onsubmit={add}>
        <Field label="Name for the new passkey (optional)"><TextInput bind:value={name} maxlength={32} placeholder="Laptop" /></Field>
        <Button type="submit" variant="primary" loading={adding}>Add a passkey</Button>
      </form>
    {/if}
  {/if}
  {#if error}<p role="alert" class="text-sm text-danger">{error}</p>{/if}
</section>

<style>
  section { display: flex; flex-direction: column; gap: 16px; }
  .keys { border: 1px solid var(--line); border-radius: var(--radius); }
  .keys > li + li { border-top: 1px solid var(--line-subtle); }
</style>
