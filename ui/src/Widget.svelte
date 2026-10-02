<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
  import { api, on, type AppInfo, type Settings, type WidgetData } from './lib/api'
  import { applyAppearance } from './lib/store.svelte'
  import WidgetView from './components/WidgetView.svelte'

  let data = $state<WidgetData | null>(null)
  let settings = $state<Settings | null>(null)
  let info: AppInfo | null = null
  let root: HTMLElement | undefined = $state()
  let host: HTMLElement | undefined = $state()

  function apply(s: Settings) {
    // the widget can have its own theme; "system" follows the app's theme setting
    applyAppearance({ ...s, theme: s.widget.theme === 'system' ? s.theme : s.widget.theme }, info)
    document.documentElement.dataset.mica = 'false'
  }

  async function load() {
    try {
      data = await api.widgetData()
    } catch {
      /* backend busy; the next tick retries */
    }
  }

  // the window follows the content size (layout, items and scale all change it)
  let lastSize = ''
  async function fit() {
    await tick()
    if (!host || !root) return
    const r = host.getBoundingClientRect()
    const w = Math.ceil(r.width)
    const h = Math.ceil(r.height)
    const key = `${w}x${h}`
    if (w > 10 && h > 10 && key !== lastSize) {
      lastSize = key
      await getCurrentWindow().setSize(new LogicalSize(w, h)).catch(() => {})
      // a widget anchored to a corner stays flush with it as its size changes
      if (settings?.widget.anchor) await api.placeWidget(settings.widget.anchor, false).catch(() => {})
    }
  }
  $effect(() => {
    void data
    void settings
    fit()
  })

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
      on<Settings>('settings-changed', (s) => {
        settings = s
        apply(s)
        load()
      }),
    ]
    const id = setInterval(load, 60_000)
    return () => {
      clearInterval(id)
      subs.forEach((p) => p.then((u) => u()))
    }
  })

  // click runs the click action; a press that moves starts a window drag (unless locked)
  let down: { x: number; y: number } | null = null
  function pointerdown(e: PointerEvent) {
    if (e.button !== 0) return
    down = { x: e.screenX, y: e.screenY }
  }
  function pointermove(e: PointerEvent) {
    if (down && Math.hypot(e.screenX - down.x, e.screenY - down.y) > 4) {
      down = null
      if (!settings?.widget.lock_position) getCurrentWindow().startDragging()
    }
  }
  function pointerup() {
    if (down && settings?.widget.click_action !== 'none') api.openMain()
    down = null
  }
  function menu(e: MouseEvent) {
    e.preventDefault()
    api.widgetMenu()
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="host" bind:this={host} onpointerdown={pointerdown} onpointermove={pointermove} onpointerup={pointerup} oncontextmenu={menu}>
  {#if settings}
    <WidgetView {data} ws={settings.widget} bind:root />
  {/if}
</div>

<style>
  :global(html.widget),
  :global(html.widget body),
  :global(html.widget #app) {
    background: transparent !important;
    overflow: hidden;
    height: auto;
  }
  .host {
    display: inline-block;
    line-height: 0;
  }
  .host :global(.w) {
    line-height: 1.35;
  }
</style>
