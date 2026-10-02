<script lang="ts">
  import { onMount } from 'svelte'
  import { app, init, type View } from './lib/store.svelte'
  import { t } from './lib/i18n.svelte'
  import Icon from './components/Icon.svelte'
  import Toolbar from './components/Toolbar.svelte'
  import Onboarding from './views/Onboarding.svelte'
  import Overview from './views/Overview.svelte'
  import Daily from './views/Daily.svelte'
  import Breakdown from './views/Breakdown.svelte'
  import Limits from './views/Limits.svelte'
  import Projects from './views/Projects.svelte'
  import Sources from './views/Sources.svelte'
  import SettingsView from './views/Settings.svelte'
  import Cache from './views/Cache.svelte'
  import WidgetStudio from './views/WidgetStudio.svelte'

  let failed = $state('')
  let page: HTMLDivElement | undefined = $state()
  // every view starts at the top
  $effect(() => {
    void app.view
    page?.scrollTo({ top: 0 })
  })
  onMount(() => {
    init().catch((e) => (failed = String(e)))
  })

  const nav: { section: string; items: { id: View; icon: string }[] }[] = [
    { section: 'nav.section.usage', items: [
      { id: 'overview', icon: 'overview' },
      { id: 'daily', icon: 'daily' },
      { id: 'breakdown', icon: 'breakdown' },
      { id: 'cache', icon: 'cache' },
      { id: 'limits', icon: 'limits' },
      { id: 'projects', icon: 'projects' },
    ] },
    { section: 'nav.section.manage', items: [
      { id: 'sources', icon: 'sources' },
      { id: 'widget', icon: 'widget' },
      { id: 'settings', icon: 'settings' },
    ] },
  ]
  const withToolbar: View[] = ['overview', 'daily', 'breakdown', 'cache', 'projects']

  function navKey(e: KeyboardEvent) {
    const flat = nav.flatMap((s) => s.items.map((i) => i.id))
    const i = flat.indexOf(app.view)
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      app.view = flat[(i + 1) % flat.length]
      document.getElementById(`nav-${app.view}`)?.focus()
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      app.view = flat[(i - 1 + flat.length) % flat.length]
      document.getElementById(`nav-${app.view}`)?.focus()
    }
  }
</script>

{#if failed}
  <main class="center"><p>{t('common.error', { e: failed })}</p></main>
{:else if !app.ready || !app.settings}
  <main class="center" aria-busy="true"><p class="muted">{t('common.loading')}</p></main>
{:else if !app.settings.onboarded}
  <Onboarding />
{:else}
  <div class="shell">
    <nav class="sidebar" aria-label="Navigation">
      <div class="brand">
        <img src="/app-icon.png" alt="" width="22" height="22" />
        <span>{t('app.name')}</span>
      </div>
      {#each nav as group (group.section)}
        <div class="section">{t(group.section)}</div>
        <ul role="list">
          {#each group.items as item (item.id)}
            <li>
              <button
                id="nav-{item.id}"
                class:active={app.view === item.id}
                aria-current={app.view === item.id ? 'page' : undefined}
                tabindex={app.view === item.id ? 0 : -1}
                onclick={() => (app.view = item.id)}
                onkeydown={navKey}
              >
                <Icon name={item.icon} size={17} />
                <span>{t(`nav.${item.id}`)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/each}
      <div class="spacer"></div>
      {#if app.scan?.running && app.scan.total > 0}
        <div class="scan" role="status">
          <div class="small muted">{t('sources.scanning', { done: app.scan.done, total: app.scan.total })}</div>
          <div class="progress"><span style="width:{(app.scan.done / Math.max(1, app.scan.total)) * 100}%"></span></div>
        </div>
      {/if}
    </nav>
    <main class="content">
      {#if withToolbar.includes(app.view)}
        <Toolbar />
      {/if}
      <div class="page" bind:this={page} aria-busy={app.loading}>
        {#if app.error}
          <div class="banner" role="alert"><Icon name="warning" size={16} />{t('common.error', { e: app.error })}</div>
        {/if}
        {#if app.view === 'overview'}<Overview />
        {:else if app.view === 'daily'}<Daily />
        {:else if app.view === 'breakdown'}<Breakdown />
        {:else if app.view === 'limits'}<Limits />
        {:else if app.view === 'projects'}<Projects />
        {:else if app.view === 'sources'}<Sources />
        {:else if app.view === 'settings'}<SettingsView />
        {:else if app.view === 'cache'}<Cache />
        {:else if app.view === 'widget'}<WidgetStudio />
        {/if}
      </div>
    </main>
  </div>
{/if}

<style>
  .center {
    height: 100%;
    display: grid;
    place-items: center;
  }
  .shell {
    display: grid;
    grid-template-columns: 228px 1fr;
    height: 100%;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    padding: 14px 10px;
    background: var(--sidebar);
    border-right: 0.5px solid var(--hairline);
    overflow-y: auto;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 4px 10px 14px;
    font-weight: 650;
    font-size: 14px;
    letter-spacing: -0.01em;
  }
  .brand img {
    border-radius: 6px;
  }
  .section {
    font-size: 11px;
    font-weight: 600;
    color: var(--ink-3);
    padding: 12px 10px 4px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--ink);
    font-size: 13.5px;
    text-align: left;
    transition: background var(--dur) var(--ease);
  }
  li button :global(svg) {
    color: var(--ink-2);
  }
  li button:hover {
    background: var(--surface-hover);
  }
  li button.active {
    background: var(--surface-press);
    font-weight: 600;
  }
  li button.active :global(svg) {
    color: var(--accent);
  }
  .spacer {
    flex: 1;
  }
  .scan {
    padding: 8px 10px;
  }
  .progress {
    height: 4px;
    border-radius: 2px;
    background: var(--surface-hover);
    margin-top: 6px;
    overflow: hidden;
  }
  .progress span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 200ms linear;
  }
  .content {
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
  }
  .page {
    flex: 1;
    overflow-y: auto;
    padding: 6px 28px 32px;
  }
</style>
