<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, errorText } from '../../lib/api';
  import { session } from '../../lib/session.svelte';
  import { copyText } from '../../lib/ui/clipboard';
  import { UI_MESSAGES as M } from '../../lib/ui/messages';
  import { loadUsers } from '../../lib/ui/users';
  import { EXPIRES, ROLE_NAMES, STATUS_NAMES, inviteStatus, untilText } from './admin';
  import Button from '../../lib/ui/c/Button.svelte';
  import Dialog from '../../lib/ui/c/Dialog.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import Select from '../../lib/ui/c/Select.svelte';
  import TextInput from '../../lib/ui/c/TextInput.svelte';
  import type { InviteCreate } from '../../lib/gen/InviteCreate';
  import type { InviteCreated } from '../../lib/gen/InviteCreated';
  import type { InviteInfo } from '../../lib/gen/InviteInfo';
  import type { Role } from '../../lib/gen/Role';
  // Admin > Invites. The server keeps only a hash of each code, so the new link lives in this component's state only:
  // shown once, never stored, gone when the page is left or Done is pressed.
  const owner = $derived(session.state.me!.user.role === 'owner');

  let role = $state<Role>('member');
  let uses = $state('1');
  let hours = $state(168);
  let creating = $state(false);
  let createError = $state('');
  let created = $state<InviteCreated | null>(null);
  let copied = $state<'' | 'ok' | 'failed'>('');
  let linkEl = $state<HTMLInputElement>();
  let form: HTMLFormElement;
  let listHead: HTMLElement;

  async function create(e: Event) {
    e.preventDefault();
    if (creating) return;
    creating = true;
    createError = '';
    try {
      const body: InviteCreate = { role: owner ? role : 'member', max_uses: Number(uses), expires_hours: hours };
      created = await api<InviteCreated>('POST', '/api/invites', body);
      copied = '';
      invites = [created.info, ...(invites ?? [])];
      now = Date.now();
      await tick();
      linkEl?.focus();
      linkEl?.select();
    } catch (err) {
      createError = errorText(err);
      await tick();
      form.querySelector<HTMLElement>('[role=alert]')?.focus();
    } finally {
      creating = false;
    }
  }
  async function copy() {
    copied = (await copyText(created!.url)) ? 'ok' : 'failed';
    if (copied === 'failed') linkEl?.select();
  }
  const dateOf = (ms: number) => new Date(ms).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: 'numeric', minute: '2-digit' });

  let invites = $state<InviteInfo[] | null>(null);
  let listFailed = $state(false);
  let now = $state(Date.now());
  async function load() {
    try {
      invites = await api<InviteInfo[]>('GET', '/api/invites'); // newest first
      listFailed = false;
      now = Date.now();
    } catch {
      listFailed = true;
    }
  }
  onMount(() => {
    void load();
    void loadUsers().catch(() => {}); // only for the Created by names; a dash stands in without them
  });
  // No creator: an owner invite from the command line (create-owner). An unknown id (users not loaded) shows a dash.
  const creator = (id: string | null) => (id === null ? 'Command line' : session.state.users[id]?.display_name ?? '-');

  let revoking = $state<InviteInfo | null>(null);
  let busy = $state(false);
  let revokeError = $state('');
  async function revoke() {
    if (busy || !revoking) return;
    busy = true;
    revokeError = '';
    const id = revoking.id;
    try {
      await api('DELETE', `/api/invites/${id}`);
      invites = invites!.map((i) => (i.id === id ? { ...i, revoked: true } : i));
      revoking = null;
      await tick();
      listHead.focus(); // the Revoke button that opened the dialog is gone with the active status
    } catch (err) {
      revokeError = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="sections">
  <section role="group" aria-labelledby="invites-new">
    <h3 id="invites-new" class="text-base font-medium text-fg">New invite</h3>
    <form onsubmit={create} bind:this={form} class="flex flex-col gap-4">
      <div class="grid max-w-140 grid-cols-3 gap-4">
        <Field label="Role" help={owner ? undefined : M.inviteModOnly}>
          <Select bind:value={role} disabled={!owner}>
            <option value="member">Member</option>
            {#if owner}<option value="moderator">Moderator</option>{/if}
          </Select>
        </Field>
        <Field label="Uses" help="1 to 50 people.">
          <TextInput type="number" inputmode="numeric" min="1" max="50" step="1" required bind:value={uses} />
        </Field>
        <Field label="Expires">
          <Select bind:value={hours}>
            {#each EXPIRES as x (x.hours)}<option value={x.hours}>{x.label}</option>{/each}
          </Select>
        </Field>
      </div>
      <div><Button type="submit" variant="primary" loading={creating}>Create invite</Button></div>
      {#if createError}<p role="alert" tabindex="-1" class="-mt-2 text-sm text-danger">{createError}</p>{/if}
    </form>

    {#if created}
      <div class="created" role="group" aria-label="New invite link">
        <div class="flex flex-col gap-1.5">
          <label for="invite-link" class="text-sm font-medium text-fg">Invite link</label>
          <div class="flex gap-2">
            <input id="invite-link" bind:this={linkEl} value={created.url} readonly onfocus={(e) => e.currentTarget.select()}
              aria-describedby="invite-once" class="control h-8 min-w-0 flex-1 px-2.5 font-mono text-xs" />
            <Button onclick={copy}>Copy link</Button>
          </div>
        </div>
        <p id="invite-once" class="text-sm text-fg-2">{M.inviteOnce(created.info.max_uses, dateOf(created.info.expires_at))}</p>
        <div class="flex items-center gap-3">
          {#if copied === 'failed'}
            <p role="alert" class="text-sm text-danger">{M.inviteCopyFailed}</p>
          {:else}
            <p role="status" class="text-sm text-fg-2">{copied === 'ok' ? M.inviteCopied : ''}</p>
          {/if}
          <span class="ml-auto"><Button variant="ghost" size="sm" onclick={() => (created = null)}>Done</Button></span>
        </div>
      </div>
    {/if}
  </section>

  <section role="group" aria-labelledby="invites-list">
    <h3 id="invites-list" tabindex="-1" bind:this={listHead} class="text-base font-medium text-fg">All invites</h3>
    {#if listFailed}
      <p role="alert" class="text-sm text-fg-2">{M.invitesFailed}</p>
    {:else if !invites}
      <p class="text-sm text-fg-2">{M.invitesLoading}</p>
    {:else if !invites.length}
      <p class="text-sm text-fg-2">{M.invitesEmpty}</p>
    {:else}
      <table class="tbl">
        <thead>
          <tr><th scope="col">Role</th><th scope="col">Uses</th><th scope="col">Expires</th><th scope="col">Created by</th><th scope="col">Status</th><th><span class="sr-only">Actions</span></th></tr>
        </thead>
        <tbody>
          {#each invites as i (i.id)}
            {@const st = inviteStatus(i, now)}
            <tr data-invite={i.id}>
              <td>{ROLE_NAMES[i.role]}</td>
              <td class="tabular-nums">{i.uses} of {i.max_uses}</td>
              <td title={new Date(i.expires_at).toLocaleString()}>{untilText(i.expires_at, now)}</td>
              <td class="who" title={creator(i.created_by)}>{creator(i.created_by)}</td>
              <td class:dim={st !== 'active'}>{STATUS_NAMES[st]}</td>
              <td class="text-right">
                {#if st === 'active'}<Button size="sm" variant="ghost" onclick={() => ([revoking, revokeError] = [i, ''])}>Revoke</Button>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<Dialog open={!!revoking} title="Revoke this invite?" onclose={() => (revoking = null)}>
  {#if revoking}
    This {ROLE_NAMES[revoking.role].toLowerCase()} link ({revoking.uses} of {revoking.max_uses} used) stops working at once. People who
    already joined with it keep their accounts.
    {#if revokeError}<p role="alert" class="mt-3 text-danger">{revokeError}</p>{/if}
  {/if}
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (revoking = null)}>Cancel</Button>
    <Button variant="danger" loading={busy} onclick={revoke}>Revoke</Button>
  {/snippet}
</Dialog>

<style>
  .sections { display: flex; flex-direction: column; gap: 32px; }
  section { display: flex; flex-direction: column; gap: 16px; }
  section + section { padding-top: 24px; border-top: 1px solid var(--line-subtle); }
  .created { display: flex; max-width: 560px; flex-direction: column; gap: 12px; padding: 16px; border: 1px solid var(--line); border-radius: var(--radius); background: var(--bg-raised); }
  /* Auto layout on one line per row: each column takes its content, so the table fits 512 px at the 1024 px window. */
  .tbl { width: 100%; border-collapse: collapse; font-size: 13px; line-height: 20px; white-space: nowrap; }
  .tbl th { height: 32px; padding: 0 8px; border-bottom: 1px solid var(--line); color: var(--fg-2); font-weight: 500; text-align: left; }
  .tbl td { height: 44px; padding: 0 8px; border-bottom: 1px solid var(--line-subtle); color: var(--fg); }
  .tbl :is(th, td):first-child { padding-left: 12px; }
  .tbl td.dim { color: var(--fg-2); }
  .tbl td.who { max-width: 160px; overflow: hidden; text-overflow: ellipsis; }
</style>
