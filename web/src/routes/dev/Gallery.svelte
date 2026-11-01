<script lang="ts">
  // Dev-only page at /_gallery: every role, the type scale, radii and the focus ring. The screenshot scene `tokens` shoots it.
  const groups: { title: string; kind: 'fill' | 'line' | 'text'; roles: string[] }[] = [
    { title: 'Surfaces', kind: 'fill', roles: ['--bg-app', '--bg-sidebar', '--bg-raised', '--bg-element', '--bg-hover', '--bg-selected'] },
    { title: 'Lines', kind: 'line', roles: ['--line-subtle', '--line', '--line-strong'] },
    { title: 'Text', kind: 'text', roles: ['--fg', '--fg-2', '--fg-3', '--fg-disabled', '--accent-fg', '--danger-fg', '--warn-fg', '--ok-fg'] },
    { title: 'Accent', kind: 'fill', roles: ['--accent', '--accent-hover', '--accent-subtle', '--accent-line', '--focus'] },
    { title: 'Status', kind: 'fill', roles: ['--danger-solid', '--danger-subtle', '--warn-subtle', '--ok-subtle'] },
    { title: 'Avatars', kind: 'fill', roles: [1, 2, 3, 4, 5, 6, 7, 8].map((n) => `--avatar-${n}`) },
  ];
  const on: Record<string, string> = { '--accent': '--on-accent', '--accent-hover': '--on-accent', '--focus': '--on-accent', '--danger-solid': '--on-danger' };
  const sizes = [
    ['text-2xl', '24/32'], ['text-xl', '20/28'], ['text-lg', '16/24'], ['text-base', '14/20, chat body'], ['text-sm', '13/20, UI default'], ['text-xs', '12/16, meta'],
  ];
  const radii = [['rounded-sm', '--radius-sm 4px'], ['rounded-md', '--radius 6px'], ['rounded-lg', '--radius-lg 10px']];
</script>

<main class="mx-auto max-w-6xl space-y-8 p-8">
  <header class="space-y-1">
    <h1 class="text-xl font-medium">Tokens</h1>
    <p class="text-fg-2">Style <code class="font-mono text-xs">vesper</code>. Components use these roles only.</p>
  </header>

  {#each groups as g (g.title)}
    <section class="space-y-3">
      <h2 class="text-xs font-medium text-fg-3 uppercase">{g.title}</h2>
      <ul class="grid grid-cols-6 gap-4">
        {#each g.roles as r (r)}
          <li class="space-y-2">
            {#if g.kind === 'text'}
              <div class="flex h-12 items-center rounded-md border border-subtle bg-app px-3 text-base font-medium" style="color: var({r})">Join voice</div>
            {:else if g.kind === 'line'}
              <div class="h-12 rounded-md bg-app" style="border: 2px solid var({r})"></div>
            {:else}
              <div class="flex h-12 items-center rounded-md border border-subtle px-3 font-medium" style="background: var({r}); color: var({on[r] ?? (r.startsWith('--avatar') ? '--on-avatar' : '--fg')})">
                {r.startsWith('--avatar') ? 'TB' : 'Aa'}
              </div>
            {/if}
            <p class="font-mono text-xs text-fg-2">{r}</p>
          </li>
        {/each}
      </ul>
    </section>
  {/each}

  <section class="space-y-3">
    <h2 class="text-xs font-medium text-fg-3 uppercase">Type</h2>
    <ul class="space-y-2">
      {#each sizes as [cls, note] (cls)}
        <li class="flex items-baseline gap-4">
          <span class="w-32 font-mono text-xs text-fg-3">{cls}</span>
          <span class={cls}>Alice joined general.</span>
          <span class="{cls} font-medium">Join voice</span>
          <span class="text-xs text-fg-3">{note}</span>
        </li>
      {/each}
      <li class="flex items-baseline gap-4">
        <span class="w-32 font-mono text-xs text-fg-3">tabular-nums</span>
        <span class="text-sm tabular-nums">10:42 . 3 new . 128 kbps . 4 % loss</span>
      </li>
      <li class="flex items-baseline gap-4">
        <span class="w-32 font-mono text-xs text-fg-3">font-mono</span>
        <code class="font-mono text-xs">vesper-server create-owner</code>
      </li>
    </ul>
  </section>

  <section class="flex gap-12">
    <div class="space-y-3">
      <h2 class="text-xs font-medium text-fg-3 uppercase">Radii</h2>
      <ul class="flex gap-4">
        {#each radii as [cls, note] (cls)}
          <li class="space-y-2">
            <div class="h-12 w-24 border border-line bg-element {cls}"></div>
            <p class="font-mono text-xs text-fg-2">{note}</p>
          </li>
        {/each}
      </ul>
    </div>
    <div class="space-y-3">
      <h2 class="text-xs font-medium text-fg-3 uppercase">Focus ring and float shadow</h2>
      <div class="flex items-center gap-6 pt-1">
        <button class="h-8 rounded-md bg-accent px-3 font-medium text-on-accent" style="outline: 2px solid var(--focus); outline-offset: 2px">Join voice</button>
        <button class="h-8 rounded-md bg-selected px-3" style="outline: 2px solid var(--focus); outline-offset: 2px">Leave</button>
        <div class="flex h-12 w-40 items-center rounded-lg border border-subtle bg-raised px-3 text-fg-2" style="box-shadow: var(--shadow-float)">Popover</div>
        <button class="h-8 rounded-md bg-element px-3" data-tip="Tooltip text">Hover for a tooltip</button>
      </div>
    </div>
  </section>
</main>
