<script lang="ts">
  import { ADMIN_NAMES as NAMES, ADMIN_OWNER_ONLY, ADMIN_SECTIONS, href, type AdminSection } from '../../lib/route';
  import { navigate, session } from '../../lib/session.svelte';
  import { UI_MESSAGES as M } from '../../lib/ui/messages';
  import Button from '../../lib/ui/c/Button.svelte';
  import EmptyState from '../../lib/ui/c/EmptyState.svelte';
  import Disk from './Disk.svelte';
  import Invites from './Invites.svelte';
  import Members from './Members.svelte';
  import Rooms from './Rooms.svelte';
  // A lazy chunk, framed like Settings: moderators get Invites and Members, the owner also Rooms and Disk use. The server
  // enforces the same rules; this only keeps a member or moderator from a page that would answer 403 everywhere.
  let { section }: { section: AdminSection } = $props();
  const role = $derived(session.state.me!.user.role);
  const sections = $derived(ADMIN_SECTIONS.filter((s) => role === 'owner' || !ADMIN_OWNER_ONLY.includes(s)));
</script>

{#if role === 'member'}
  {@render denied(M.adminDenied)}
{:else}
  <div class="flex min-h-0 flex-1 justify-center gap-8 overflow-y-auto px-4 py-6">
    <nav aria-label="Admin sections" class="sticky top-0 w-50 shrink-0 self-start">
      <ul class="flex flex-col gap-px">
        {#each sections as s (s)}
          <li>
            <a href={href({ name: 'admin', section: s })} aria-current={s === section ? 'page' : undefined}
              class="flex h-8 items-center rounded-sm px-2 text-fg-2 hover:bg-hover hover:text-fg aria-[current=page]:bg-selected aria-[current=page]:text-fg">{NAMES[s]}</a>
          </li>
        {/each}
      </ul>
    </nav>
    <div class="min-w-0 max-w-200 flex-1 pb-12">
      {#if !sections.includes(section)}
        {@render denied(M.adminOwnerOnly)}
      {:else}
        <h2 class="mb-6 text-lg font-medium text-fg">{NAMES[section]}</h2>
        {#if section === 'invites'}
          <Invites />
        {:else if section === 'members'}
          <Members />
        {:else if section === 'rooms'}
          <Rooms />
        {:else}
          <Disk />
        {/if}
      {/if}
    </div>
  </div>
{/if}

{#snippet denied(text: string)}
  <div class="flex flex-1 flex-col justify-center pb-12">
    <EmptyState title={text}>
      {#snippet action()}<Button onclick={() => navigate('/')}>{M.goHome}</Button>{/snippet}
    </EmptyState>
  </div>
{/snippet}
