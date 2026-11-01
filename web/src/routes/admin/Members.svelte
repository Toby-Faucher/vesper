<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, ApiFailure, errorText } from '../../lib/api';
  import { session } from '../../lib/session.svelte';
  import { UI_MESSAGES as M } from '../../lib/ui/messages';
  import { copyText } from '../../lib/ui/clipboard';
  import { ROLE_NAMES, byRole, canReset, resetLine } from './admin';
  import Avatar from '../../lib/ui/c/Avatar.svelte';
  import Button from '../../lib/ui/c/Button.svelte';
  import PersonButton from '../../lib/ui/c/PersonButton.svelte';
  import Dialog from '../../lib/ui/c/Dialog.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import PasswordInput from '../../lib/ui/c/PasswordInput.svelte';
  import Select from '../../lib/ui/c/Select.svelte';
  import type { ResetCreate } from '../../lib/gen/ResetCreate';
  import type { ResetCreated } from '../../lib/gen/ResetCreated';
  import type { ResetInfo } from '../../lib/gen/ResetInfo';
  import type { Role } from '../../lib/gen/Role';
  import type { RolePatch } from '../../lib/gen/RolePatch';
  import type { UserInfo } from '../../lib/gen/UserInfo';
  // Admin > Members. Rows come from the session's user map, which user_created and user_updated keep current (a signup,
  // a role change or a ban made elsewhere shows here live); the fetch on open is for the loaded and failed states.
  // Role and ban are the owner's, on anyone but themself; the server checks the same (owner_target).
  const me = $derived(session.state.me!.user);
  const owner = $derived(me.role === 'owner');
  let loaded = $state(false);
  let failed = $state(false);
  let loading = $state(false);
  let tableEl = $state<HTMLElement>();
  async function load() {
    if (loading) return;
    loading = true;
    try {
      const list = await api<UserInfo[]>('GET', '/api/users');
      session.state.users = { ...session.state.users, ...Object.fromEntries(list.map((u) => [u.user_id, u])) };
      [loaded, failed] = [true, false];
    } catch {
      failed = !loaded; // a failed reload keeps the table
    } finally {
      loading = false;
    }
  }
  // Try again unmounts its button on success: focus goes to the table, not the body.
  async function retry() {
    await load();
    await tick();
    if (!failed) tableEl?.focus();
  }
  onMount(() => {
    void load();
    void loadResets();
  });
  const members = $derived(Object.values(session.state.users).sort(byRole));
  const editable = (u: UserInfo) => owner && u.user_id !== me.user_id && u.role !== 'owner';

  let error = $state('');
  let errorEl = $state<HTMLElement>();
  let busy = $state('');
  async function send(u: UserInfo, method: 'PATCH' | 'POST', path: string, body?: RolePatch) {
    if (busy) return false;
    busy = u.user_id;
    error = '';
    try {
      const fresh = await api<UserInfo>(method, `/api/users/${u.user_id}/${path}`, body);
      session.state.users = { ...session.state.users, [fresh.user_id]: fresh };
      return true;
    } catch (e) {
      error = errorText(e);
      await tick();
      errorEl?.focus();
      return false;
    } finally {
      busy = '';
    }
  }

  // A role change and a ban ask first and name the person; Cancel puts the select back on the stored role.
  let confirm = $state<{ u: UserInfo; role?: Role; select?: HTMLSelectElement } | null>(null);
  function pickRole(u: UserInfo, e: Event) {
    const select = e.currentTarget as HTMLSelectElement;
    if (select.value !== u.role) confirm = { u, role: select.value as Role, select };
  }
  // Not at pick time: Select's own binding would set the new role again right after.
  function cancel() {
    if (confirm?.select) confirm.select.value = confirm.u.role;
    confirm = null;
  }
  async function confirmed() {
    if (!confirm) return;
    const { u, role } = confirm;
    const ok = role ? await send(u, 'PATCH', 'role', { role }) : await send(u, 'POST', 'ban');
    if (ok) confirm = null;
  }

  // Reset links: confirm naming the person and asking for your own password, then the link once, like a new invite
  // (only in this state, never stored). The owner also sees each person's latest reset: who made it and if it was used.
  let resets = $state<Record<string, ResetInfo>>({});
  async function loadResets() {
    if (!owner) return;
    try {
      resets = Object.fromEntries((await api<ResetInfo[]>('GET', '/api/resets')).map((r) => [r.user_id, r]));
    } catch {
      // only the audit line; the table works without it
    }
  }
  const resetBy = (r: ResetInfo) => (r.created_by === null ? null : (session.state.users[r.created_by]?.display_name ?? 'someone'));
  let resetPassword = $state('');
  let resetFor = $state<UserInfo | null>(null);
  let resetError = $state('');
  let resetBusy = $state(false);
  let made = $state<(ResetCreated & { name: string }) | null>(null);
  let copied = $state<'' | 'ok' | 'failed'>('');
  let linkEl = $state<HTMLInputElement>();
  async function makeReset() {
    if (!resetFor || resetBusy) return;
    resetBusy = true;
    resetError = '';
    const u = resetFor;
    try {
      const body: ResetCreate = { password: resetPassword };
      made = { ...(await api<ResetCreated>('POST', `/api/users/${u.user_id}/reset`, body)), name: u.display_name };
      [copied, resetFor] = ['', null];
      void loadResets();
      await tick();
      linkEl?.focus();
    } catch (e) {
      resetError = e instanceof ApiFailure && e.code === 'forbidden' && e.detail === 'wrong password' ? M.resetWrongPassword : errorText(e);
    } finally {
      resetPassword = '';
      resetBusy = false;
    }
  }
  async function copy() {
    copied = (await copyText(made!.url)) ? 'ok' : 'failed';
    if (copied === 'failed') linkEl?.select();
  }
  const dateOf = (ms: number) => new Date(ms).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: 'numeric', minute: '2-digit' });
</script>

{#if !owner}<p class="mb-4 text-sm text-fg-2">{M.membersOwnerOnly}</p>{/if}
{#if failed}
  <div class="flex items-center gap-3">
    <p role="alert" class="text-sm text-fg-2">{M.membersFailed}</p>
    <Button size="sm" {loading} onclick={retry}>Try again</Button>
  </div>
{:else if !loaded}
  <p class="text-sm text-fg-2">{M.membersLoading}</p>
{:else}
  <table class="tbl table-fixed" aria-label="Members" tabindex="-1" bind:this={tableEl}>
    <thead>
      <tr><th scope="col">Name</th><th scope="col" class="w-36">Role</th><th scope="col" class="w-20">Status</th><th class="w-40"><span class="sr-only">Actions</span></th></tr>
    </thead>
    <tbody>
      {#each members as u (u.user_id)}
        <tr data-user={u.username}>
          <td>
            <!-- The card has the voice actions for a moderator. -->
            <PersonButton id={u.user_id} name={u.display_name} class="flex! max-w-full min-w-0 items-center gap-3 hover:no-underline!">
              <Avatar user={u} size={20} />
              <span class="min-w-0">
                <span class="block truncate text-fg hover:underline" title={u.display_name}>{u.display_name}</span>
                <span class="block truncate text-xs text-fg-2">@{u.username}{u.user_id === me.user_id ? ' (you)' : ''}</span>
              </span>
            </PersonButton>
            {#if resets[u.user_id]}{@const line = resetLine(resets[u.user_id], resetBy(resets[u.user_id]), Date.now())}
              <p class="mt-0.5 pl-8 text-xs text-fg-2">{line}</p>{/if}
          </td>
          <td>
            {#if editable(u)}
              <Select value={u.role} aria-label="Role for {u.display_name}" onchange={(e) => pickRole(u, e)}>
                <option value="member">Member</option>
                <option value="moderator">Moderator</option>
              </Select>
            {:else}
              {ROLE_NAMES[u.role]}
            {/if}
          </td>
          <td class="text-fg-2">{u.banned ? 'Banned' : ''}</td>
          <td>
            <div class="flex flex-wrap justify-end gap-2">
              {#if canReset(me, u)}
                <Button size="sm" variant="ghost" aria-label="Create reset link for {u.display_name}" aria-haspopup="dialog"
                  onclick={() => ([resetFor, resetError, resetPassword] = [u, '', ''])}>Reset link</Button>
              {/if}
              {#if editable(u)}
                {#if u.banned}
                  <Button size="sm" loading={busy === u.user_id} aria-label="Unban {u.display_name}" onclick={() => send(u, 'POST', 'unban')}>Unban</Button>
                {:else}
                  <Button size="sm" variant="ghost" aria-label="Ban {u.display_name}" onclick={() => ([confirm, error] = [{ u }, ''])}>Ban</Button>
                {/if}
              {/if}
            </div>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
  {#if made}
    <div class="created" role="group" aria-label="Reset link for {made.name}">
      <div class="flex flex-col gap-1.5">
        <label for="reset-link" class="text-sm font-ui text-fg">Reset link for {made.name}</label>
        <div class="flex gap-2">
          <input id="reset-link" bind:this={linkEl} value={made.url} readonly onfocus={(e) => e.currentTarget.select()}
            aria-describedby="reset-once" class="control h-8 min-w-0 flex-1 px-2.5 font-mono text-xs" />
          <Button onclick={copy}>Copy link</Button>
        </div>
      </div>
      <p id="reset-once" class="text-sm text-fg-2">{M.resetOnce(dateOf(made.expires_at))}</p>
      <div class="flex items-center gap-3">
        {#if copied === 'failed'}
          <p role="alert" class="text-sm text-danger">{M.inviteCopyFailed}</p>
        {:else}
          <p role="status" class="text-sm text-fg-2">{copied === 'ok' ? M.inviteCopied : ''}</p>
        {/if}
        <span class="ml-auto"><Button variant="ghost" size="sm" onclick={() => (made = null)}>Done</Button></span>
      </div>
    </div>
  {/if}
  {#if error && !confirm}<p bind:this={errorEl} role="alert" tabindex="-1" class="mt-3 text-sm text-danger">{error}</p>{/if}
{/if}

<Dialog open={!!confirm} title={confirm ? (confirm.role ? `Make ${confirm.u.display_name} a ${ROLE_NAMES[confirm.role].toLowerCase()}?` : `Ban ${confirm.u.display_name}?`) : ''}
  onclose={cancel}>
  {#if confirm?.role === 'moderator'}
    Moderators can create member invites, see every room, and mute or remove members in voice.
  {:else if confirm?.role}
    They lose the moderator tools and the restricted rooms they were not listed in.
  {:else}
    {M.banBody}
  {/if}
  {#if error && confirm}<p bind:this={errorEl} role="alert" tabindex="-1" class="mt-3 text-danger">{error}</p>{/if}
  {#snippet footer()}
    <Button variant="ghost" onclick={cancel}>Cancel</Button>
    <Button variant={confirm?.role ? 'primary' : 'danger'} loading={!!busy} onclick={confirmed}>{confirm?.role ? 'Change role' : 'Ban'}</Button>
  {/snippet}
</Dialog>

<Dialog open={!!resetFor} title={resetFor ? `Create a reset link for ${resetFor.display_name}?` : ''} onclose={() => (resetFor = null)}>
  {M.resetBody}
  {#if resetFor}
    <form id="reset-form" class="mt-4" onsubmit={(e) => (e.preventDefault(), void makeReset())}>
      <Field label="Your password" help="To check it is you.">
        <PasswordInput autocomplete="current-password" bind:value={resetPassword} required />
      </Field>
    </form>
  {/if}
  {#if resetError}<p role="alert" tabindex="-1" class="mt-3 text-danger">{resetError}</p>{/if}
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (resetFor = null)}>Cancel</Button>
    <Button variant="primary" type="submit" form="reset-form" loading={resetBusy}>Create link</Button>
  {/snippet}
</Dialog>

<style>
  .created { display: flex; max-width: 560px; margin-top: 16px; flex-direction: column; gap: 12px; padding: 16px; border: 1px solid var(--line); border-radius: var(--radius); background: var(--bg-raised); }
</style>
