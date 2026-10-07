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
  import Sessions from './views/Sessions.svelte'
  import Context from './views/Context.svelte'
  import Tips from './views/Tips.svelte'
  import { ask } from '@tauri-apps/plugin-dialog'
  import { api } from './lib/api'

  let failed = $state('')

  // short incremental scans run every few seconds while a tool writes; only a long one is worth showing
  const scanning = $derived(!!(app.scan?.running && app.scan.total > 0))
  let showScan = $state(false)
  $effect(() => {
    if (!scanning) {
      showScan = false
      return
    }
    const timer = setTimeout(() => (showScan = true), 1000)
    return () => clearTimeout(timer)
  })
  let page: HTMLDivElement | undefined = $state()
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
      { id: 'tips', icon: 'tips' },
      { id: 'daily', icon: 'daily' },
      { id: 'breakdown', icon: 'breakdown' },
      { id: 'sessions', icon: 'sessions' },
      { id: 'cache', icon: 'cache' },
      { id: 'context', icon: 'context' },
      { id: 'limits', icon: 'limits' },
      { id: 'projects', icon: 'projects' },
    ] },
    { section: 'nav.section.manage', items: [
      { id: 'sources', icon: 'sources' },
      { id: 'widget', icon: 'widget' },
      { id: 'settings', icon: 'settings' },
    ] },
  ]
  const withToolbar: View[] = ['overview', 'tips', 'daily', 'breakdown', 'sessions', 'cache', 'context', 'projects']

  // dismissed for this run only (not persisted)
  let updateDismissed = $state('')
  const update = $derived(app.update?.available && app.update.available.version !== updateDismissed ? app.update.available : null)
  async function installUpdate(v: string) {
    if (!(await ask(t('settings.updates.confirm', { v }), { title: t('settings.updates'), kind: 'info' }))) return
    try {
      await api.installUpdate()
    } catch (e) {
      app.error = String(e) === 'update_install_manual' ? t('settings.updates.manual') : String(e)
    }
  }

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
    <nav class="sidebar" aria-label={t('nav.label')}>
      <div class="brand">
        <img src="/app-icon.png" alt="" width="22" height="22" />
        <span class="label">{t('app.name')}</span>
      </div>
      {#each nav as group (group.section)}
        <div class="section"><span class="label">{t(group.section)}</span></div>
        <ul role="list">
          {#each group.items as item (item.id)}
            <li>
              <button
                id="nav-{item.id}"
                class:active={app.view === item.id}
                aria-current={app.view === item.id ? 'page' : undefined}
                tabindex={app.view === item.id ? 0 : -1}
                title={t(`nav.${item.id}`)}
                onclick={() => (app.view = item.id)}
                onkeydown={navKey}
              >
                <Icon name={item.icon} size={17} />
                <span class="label">{t(`nav.${item.id}`)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/each}
      <div class="spacer"></div>
      {#if showScan && app.scan}
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
        {#if update && app.view !== 'settings'}
          <div class="banner update" role="status">
            <Icon name="download" size={16} />
            <span>{t('update.banner', { v: update.version })}</span>
            <span class="grow"></span>
            <button class="btn ghost small" onclick={() => (app.view = 'settings')}>{t('update.banner.details')}</button>
            <button class="btn primary small" disabled={app.update?.installing} onclick={() => installUpdate(update.version)}>{t('update.banner.install')}</button>
            <button class="btn ghost small" aria-label={t('common.close')} onclick={() => (updateDismissed = update.version)}><Icon name="close" size={14} /></button>
          </div>
        {/if}
        {#if app.view === 'overview'}<Overview />
        {:else if app.view === 'tips'}<Tips />
        {:else if app.view === 'daily'}<Daily />
        {:else if app.view === 'breakdown'}<Breakdown />
        {:else if app.view === 'limits'}<Limits />
        {:else if app.view === 'projects'}<Projects />
        {:else if app.view === 'sources'}<Sources />
        {:else if app.view === 'settings'}<SettingsView />
        {:else if app.view === 'cache'}<Cache />
        {:else if app.view === 'sessions'}<Sessions />
        {:else if app.view === 'context'}<Context />
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
    grid-template-columns: 228px minmax(0, 1fr);
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
    container: main / inline-size;
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
  /* icon-only sidebar; names remain as tooltips and, visually hidden, for screen readers */
  @media (max-width: 1100px) {
    .shell {
      grid-template-columns: 60px minmax(0, 1fr);
    }
    .sidebar {
      padding: 14px 8px;
    }
    .sidebar .label {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0 0 0 0);
      white-space: nowrap;
    }
    .brand {
      justify-content: center;
      padding: 4px 0 10px;
    }
    .section {
      height: 0;
      padding: 0;
      margin: 10px 6px;
      border-top: 0.5px solid var(--hairline);
    }
    li button {
      justify-content: center;
      padding: 0;
      height: 36px;
    }
    .scan .small {
      display: none;
    }
    .scan {
      padding: 8px 4px;
    }
    .page {
      padding: 6px 20px 28px;
    }
  }
  .banner.update {
    align-items: center;
  }
  .grow {
    flex: 1;
  }
</style>
