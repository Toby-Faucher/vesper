<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, errorText } from '../../lib/api';
  import { session } from '../../lib/session.svelte';
  import { UI_MESSAGES as M } from '../../lib/ui/messages';
  import { canModerate, togglePerson } from '../../lib/ui/person.svelte';
  import { ROLE_NAMES, byRole } from './admin';
  import Avatar from '../../lib/ui/c/Avatar.svelte';
  import Button from '../../lib/ui/c/Button.svelte';
  import Dialog from '../../lib/ui/c/Dialog.svelte';
  import Select from '../../lib/ui/c/Select.svelte';
  import type { Role } from '../../lib/gen/Role';
  import type { RolePatch } from '../../lib/gen/RolePatch';
  import type { UserInfo } from '../../lib/gen/UserInfo';
  // Admin > Members. Rows come from the session's user map, which user_updated keeps current (a role change or a ban
  // made in another tab shows here too); the fetch on open adds people who signed up after this tab connected.
  // Role and ban are the owner's, on anyone but themself; the server checks the same (owner_target).
  const me = $derived(session.state.me!.user);
  const owner = $derived(me.role === 'owner');
  let loaded = $state(false);
  let failed = $state(false);
  onMount(async () => {
    try {
      const list = await api<UserInfo[]>('GET', '/api/users');
      session.state.users = { ...session.state.users, ...Object.fromEntries(list.map((u) => [u.user_id, u])) };
      loaded = true;
    } catch {
      failed = true;
    }
  });
  const members = $derived(Object.values(session.state.users).sort(byRole));
  const editable = (u: UserInfo) => owner && u.user_id !== me.user_id && u.role !== 'owner';
  const inVoice = (u: UserInfo) => session.state.voice.some((p) => p.user_id === u.user_id);

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
</script>

{#if !owner}<p class="mb-4 text-sm text-fg-2">{M.membersOwnerOnly}</p>{/if}
{#if failed}
  <p role="alert" class="text-sm text-fg-2">{M.membersFailed}</p>
{:else if !loaded}
  <p class="text-sm text-fg-2">{M.membersLoading}</p>
{:else}
  <table class="tbl" aria-label="Members">
    <thead>
      <tr><th scope="col">Name</th><th scope="col" class="w-36">Role</th><th scope="col" class="w-20">Status</th><th class="w-36"><span class="sr-only">Actions</span></th></tr>
    </thead>
    <tbody>
      {#each members as u (u.user_id)}
        <tr data-user={u.username}>
          <td>
            <div class="flex min-w-0 items-center gap-3">
              <Avatar user={u} size={20} />
              <div class="min-w-0">
                <p class="truncate text-fg" title={u.display_name}>{u.display_name}</p>
                <p class="truncate text-xs text-fg-2">@{u.username}{u.user_id === me.user_id ? ' (you)' : ''}</p>
              </div>
            </div>
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
            <div class="flex justify-end gap-2">
              {#if canModerate(me, u) && inVoice(u) && !u.banned}
                <Button size="sm" variant="ghost" aria-label="Voice actions for {u.display_name}" aria-haspopup="dialog"
                  onclick={(e) => togglePerson(u.user_id, e.currentTarget)}>Voice</Button>
              {/if}
              {#if editable(u)}
                {#if u.banned}
                  <Button size="sm" loading={busy === u.user_id} aria-label="Unban {u.display_name}" onclick={() => send(u, 'POST', 'unban')}>Unban</Button>
                {:else}
                  <Button size="sm" variant="danger-ghost" aria-label="Ban {u.display_name}" onclick={() => ([confirm, error] = [{ u }, ''])}>Ban</Button>
                {/if}
              {/if}
            </div>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
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

<style>
  .tbl { width: 100%; table-layout: fixed; border-collapse: collapse; font-size: 13px; line-height: 20px; }
  .tbl th { height: 32px; padding: 0 12px; border-bottom: 1px solid var(--line); color: var(--fg-2); font-weight: 500; text-align: left; white-space: nowrap; }
  .tbl td { height: 48px; padding: 0 12px; border-bottom: 1px solid var(--line-subtle); color: var(--fg); }
</style>
