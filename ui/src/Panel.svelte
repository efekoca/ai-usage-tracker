<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
  import { api, on, type AppInfo, type PanelData, type Settings } from './lib/api'
  import { applyAppearance } from './lib/store.svelte'
  import { fmtClock, t } from './lib/i18n.svelte'
  import PanelGrid from './components/panel/PanelGrid.svelte'
  import Icon from './components/Icon.svelte'

  let data = $state<PanelData | null>(null)
  let settings = $state<Settings | null>(null)
  let info = $state<AppInfo | null>(null)
  let host: HTMLElement | undefined = $state()
  let now = $state(Date.now())
  let loadedAt = $state(Date.now())

  // the system's glass on macOS and Windows 11; a plain surface elsewhere
  const material = $derived(info?.platform === 'macos' || info?.supports_mica ? 'native' : 'solid')

  function apply(s: Settings) {
    applyAppearance(s, info)
    document.documentElement.dataset.mica = 'false'
  }

  let gen = 0
  async function load() {
    const mine = ++gen
    try {
      const d = await api.panelData()
      if (mine === gen) {
        data = d
        now = loadedAt = Date.now()
      }
    } catch {
      /* backend busy; the next refresh retries */
    }
  }

  let lastSize = ''
  async function fit() {
    await tick()
    if (!host) return
    const r = host.getBoundingClientRect()
    const key = `${Math.ceil(r.width)}x${Math.ceil(r.height)}`
    if (r.width > 10 && r.height > 10 && key !== lastSize) {
      lastSize = key
      try {
        await getCurrentWindow().setSize(new LogicalSize(Math.ceil(r.width), Math.ceil(r.height)))
      } catch {
        /* no window to size (the browser preview) */
      }
    }
  }
  $effect(() => {
    void data
    void settings?.panel.modules
    fit()
  })

  // a scan with nothing new ends in a moment, after the worker's short wait: the button spins from
  // the click until the scan is over, at least long enough to be seen, then shows it is done
  let busy = $state(false)
  let done = $state(false)
  let minUntil = 0
  let timers: ReturnType<typeof setTimeout>[] = []
  function rescan() {
    timers.forEach(clearTimeout)
    busy = true
    done = false
    minUntil = Date.now() + 900
    timers = [setTimeout(finished, 20_000)]
    api.panelAction('rescan').catch(finished)
  }
  function finished() {
    if (!busy) return
    timers.forEach(clearTimeout)
    timers = [
      setTimeout(() => {
        busy = false
        done = true
        timers.push(setTimeout(() => (done = false), 1600))
      }, Math.max(0, minUntil - Date.now())),
    ]
  }
  const view = $derived(data ? { ...data, scanning: data.scanning || busy } : null)

  const act = (a: string) => (a === 'rescan' ? rescan() : api.panelAction(a).catch(() => {}))

  onMount(() => {
    ;(async () => {
      const [i, s] = await Promise.all([api.appInfo(), api.getSettings()])
      info = i
      settings = s
      apply(s)
      await load()
    })()
    const subs = [
      on('data-changed', load),
      on('scan-progress', () => data && !data.scanning && load()),
      on('scan-finished', () => {
        load()
        finished()
      }),
      on('update-status', load),
      on<Settings>('settings-changed', (s) => {
        settings = s
        apply(s)
        load()
      }),
    ]
    // opened again: show fresh values at once
    const focus = () => load()
    window.addEventListener('focus', focus)
    const keys = (e: KeyboardEvent) => e.key === 'Escape' && act('close')
    window.addEventListener('keydown', keys)
    const clock = setInterval(() => (now = Date.now()), 15_000)
    const refresh = setInterval(() => document.hasFocus() && load(), 60_000)
    const ro = new ResizeObserver(() => fit())
    if (host) ro.observe(host)
    return () => {
      ro.disconnect()
      timers.forEach(clearTimeout)
      clearInterval(clock)
      clearInterval(refresh)
      window.removeEventListener('focus', focus)
      window.removeEventListener('keydown', keys)
      subs.forEach((p) => p.then((u) => u()))
    }
  })
</script>

<div class="host" bind:this={host} data-material={material}>
  {#if settings}
    <section class="cc" aria-label={t('panel.label')}>
      {#if data?.update}
        <button class="update" onclick={() => act('update')}>
          <span class="dot"><Icon name="download" size={14} /></span>
          <span class="grow">{t('panel.update', { v: data.update })}</span>
          <span class="go">{t('panel.update.install')}</span>
        </button>
      {/if}
      {#each data?.signed_out ?? [] as p (p)}
        <button class="notice" onclick={() => act('limits')}>
          <span class="dot"><Icon name="warning" size={14} /></span>
          <span class="grow">{t(`panel.signedOut.${p}`)}</span>
          <Icon name="chevron" size={12} />
        </button>
      {/each}
      {#if settings.panel.modules.length}
        <PanelGrid modules={settings.panel.modules} data={view} {settings} {now} scanned={done} onaction={act} />
      {:else}
        <p class="empty">{t('panel.empty')}</p>
      {/if}
      <footer>
        <span class="when">{data ? t('widget.updated', { t: fmtClock(loadedAt) }) : ''}</span>
      </footer>
    </section>
  {/if}
</div>

<style>
  :global(html.panel),
  :global(html.panel body),
  :global(html.panel #app) {
    background: transparent !important;
    overflow: hidden;
    height: auto;
    -webkit-user-select: none;
    user-select: none;
    cursor: default;
  }
  .host {
    display: inline-block;
  }
  .cc {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 304px;
    padding: 12px;
  }
  .host[data-material='solid'] .cc {
    border-radius: 16px;
    border: 0.5px solid var(--hairline-strong);
    background: var(--window);
  }
  .host[data-material='solid'] .cc :global(.mod) {
    background: var(--surface);
  }
  .update {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 44px;
    padding: 0 8px 0 10px;
    border: 0;
    border-radius: 14px;
    background: var(--accent);
    color: var(--accent-ink);
    font: inherit;
    font-size: 12.5px;
    font-weight: 600;
    text-align: left;
  }
  .update:hover {
    filter: brightness(1.06);
  }
  .update .dot {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent-ink) 20%, transparent);
  }
  .update .go {
    padding: 5px 10px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent-ink) 20%, transparent);
  }
  .grow {
    flex: 1;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 9px;
    min-height: 40px;
    padding: 7px 10px;
    border: 0.5px solid color-mix(in srgb, var(--warning) 40%, transparent);
    border-radius: 14px;
    background: color-mix(in srgb, var(--warning) 14%, var(--cc-tile));
    color: var(--ink);
    font: inherit;
    font-size: 11.5px;
    line-height: 1.3;
    text-align: left;
  }
  .notice:hover {
    background: color-mix(in srgb, var(--warning) 22%, var(--cc-tile));
  }
  .notice .dot {
    display: grid;
    place-items: center;
    flex: none;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--warning) 30%, transparent);
  }
  .notice:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .empty {
    margin: 0;
    padding: 22px 12px;
    border-radius: 18px;
    background: var(--cc-tile);
    color: var(--ink-2);
    font-size: 12.5px;
    text-align: center;
  }
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 0 2px;
  }
  .when {
    font-size: 11px;
    color: var(--ink-3);
    font-variant-numeric: tabular-nums;
  }
  .update:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
