<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import { app, latest, saveSettings } from '../lib/store.svelte'
  import { api, type PanelData, type PanelModule, type PanelSize, type Provider, type TrayItem, type TrayPreview, type TraySettings } from '../lib/api'
  import { fmtClock, fmtPct, limitName, t, windowLabel } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import Select from '../components/Select.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Icon from '../components/Icon.svelte'
  import BrandIcon from '../components/BrandIcon.svelte'
  import PanelGrid from '../components/panel/PanelGrid.svelte'
  import PanelModuleView, { moduleKey } from '../components/panel/PanelModule.svelte'

  const platform = $derived(app.info?.platform ?? 'windows')
  const windows = $derived(platform === 'windows')
  const mac = $derived(platform === 'macos')

  // only what is edited here is saved, so a stale copy never overwrites newer settings
  let tray = $state<TraySettings>(structuredClone($state.snapshot(app.settings!.tray)) as TraySettings)
  let modules = $state<PanelModule[]>(structuredClone($state.snapshot(app.settings!.panel.modules)) as PanelModule[])
  type Key = keyof TraySettings
  const PANEL = 'panel'
  const pending = new Map<Key | typeof PANEL, number>()
  let edits = 0
  $effect(() => {
    const s = app.settings
    if (!s) return
    const saved = $state.snapshot(s.tray) as TraySettings
    const savedModules = $state.snapshot(s.panel.modules) as PanelModule[]
    untrack(() => {
      for (const k of Object.keys(saved) as Key[]) if (!pending.has(k)) (tray as Record<Key, unknown>)[k] = structuredClone(saved[k])
      if (!pending.has(PANEL)) modules = structuredClone(savedModules)
    })
  })
  let timer: ReturnType<typeof setTimeout> | undefined
  function flush() {
    clearTimeout(timer)
    timer = undefined
    if (!pending.size) return
    const marks = [...pending]
    const trayPatch = Object.fromEntries(marks.filter(([k]) => k !== PANEL).map(([k]) => [k, $state.snapshot(tray[k as Key])])) as Partial<TraySettings>
    const panel = pending.has(PANEL) ? { modules: $state.snapshot(modules) as PanelModule[], value: panelValue } : null
    saveSettings((c) => ({ tray: { ...c.tray, ...trayPatch }, ...(panel ? { panel } : {}) }))
      .catch(() => {})
      .finally(() => {
        for (const [k, n] of marks) if (pending.get(k) === n) pending.delete(k)
      })
  }
  function later(k: Key | typeof PANEL) {
    pending.set(k, ++edits)
    clearTimeout(timer)
    timer = setTimeout(flush, 250)
  }
  function set<K extends Key>(k: K, v: TraySettings[K]) {
    tray[k] = v
    later(k)
  }
  function setModules(list: PanelModule[]) {
    modules = list
    later(PANEL)
  }
  onDestroy(flush)

  // live data, from the same commands the panel and the menu bar use
  let data = $state<PanelData | null>(null)
  let preview = $state<TrayPreview | null>(null)
  let now = $state(Date.now())
  $effect(() => {
    void app.tick
    return latest(
      () => api.panelData(),
      (d) => {
        data = d
        now = Date.now()
      },
      () => {},
    )
  })
  $effect(() => {
    void app.tick
    const snap = $state.snapshot(tray) as TraySettings
    return latest(() => api.trayPreview(snap), (p) => (preview = p), () => {})
  })

  // the gallery: every module, as in `panel_catalog` (settings.rs)
  const MAX = 16
  const SIZES: Record<string, PanelSize[]> = { limit: ['w', 's', 'm'], provider: ['m', 'w', 'l'], usage: ['w', 's', 'm'], forecast: ['w', 'l'], scan: ['w'], action: ['s', 'w'] }
  const ORDER: PanelSize[] = ['s', 'w', 'm', 'l']
  const POOLS = [['anthropic', ''], ['openai', ''], ['google', 'gemini'], ['google', '3p']] as const
  const mk = (m: Partial<PanelModule> & { kind: PanelModule['kind'] }): PanelModule => ({ provider: '', pool: '', window: '', period: '', action: '', size: SIZES[m.kind][0], style: '', ...m })
  const catalog: PanelModule[] = [
    ...POOLS.map(([provider, pool]) => mk({ kind: 'provider', provider, pool, style: 'ring' })),
    ...POOLS.flatMap(([provider, pool]) => ['five_hour', 'seven_day'].map((window) => mk({ kind: 'limit', provider, pool, window, style: 'ring' }))),
    ...POOLS.flatMap(([provider, pool]) => ['five_hour', 'seven_day'].map((window) => mk({ kind: 'forecast', provider, pool, window }))),
    ...['today', 'days7', 'month1'].map((period) => mk({ kind: 'usage', period })),
    mk({ kind: 'scan' }),
    ...['open', 'widget', 'rescan', 'settings'].map((action) => mk({ kind: 'action', action })),
  ]
  const defaults = (): PanelModule[] => {
    const find = (f: (m: PanelModule) => boolean) => ({ ...catalog.find(f)! })
    return [
      find((m) => m.kind === 'provider' && m.provider === 'anthropic'),
      find((m) => m.kind === 'provider' && m.provider === 'openai'),
      find((m) => m.kind === 'usage' && m.period === 'today'),
      find((m) => m.kind === 'forecast' && m.provider === 'anthropic' && m.window === 'five_hour'),
      ...['open', 'widget', 'rescan', 'settings'].map((a) => find((m) => m.action === a)),
    ]
  }
  const category = (m: PanelModule) => (m.kind === 'provider' || m.kind === 'limit' ? 'limits' : m.kind === 'scan' || m.kind === 'action' ? 'controls' : m.kind)
  const modName = (m: PanelModule) => {
    const who = limitName(m.provider, m.pool)
    switch (m.kind) {
      case 'provider':
        return `${who} · ${t('menubar.mod.both')}`
      case 'limit':
        return `${who} · ${windowLabel(m.window)}`
      case 'forecast':
        return t('menubar.mod.forecast', { name: `${who} · ${windowLabel(m.window)}` })
      case 'usage':
        return `${t('menubar.mod.usage')} · ${t(`widget.period.${m.period}`)}`
      case 'scan':
        return t('panel.scan')
      default:
        return t(`panel.action.${m.action}`)
    }
  }
  const actionIcon: Record<string, string> = { open: 'overview', widget: 'widget', rescan: 'refresh', settings: 'settings' }
  const has = (m: PanelModule) => modules.some((x) => moduleKey(x) === moduleKey(m))

  let cat = $state('all')
  let query = $state('')
  const groups = $derived.by(() => {
    const q = query.trim().toLocaleLowerCase()
    const shown = catalog.filter((m) => (cat === 'all' || category(m) === cat) && modName(m).toLocaleLowerCase().includes(q))
    return (['limits', 'forecast', 'usage', 'controls'] as const).map((c) => ({ c, items: shown.filter((m) => category(m) === c) })).filter((g) => g.items.length)
  })

  // editing
  let selected = $state<string | null>(null)
  const selIndex = $derived(modules.findIndex((m) => moduleKey(m) === selected))
  const sel = $derived(selIndex >= 0 ? modules[selIndex] : null)

  function add(m: PanelModule, at = modules.length) {
    const rest = modules.filter((x) => moduleKey(x) !== moduleKey(m))
    if (rest.length >= MAX) return
    rest.splice(Math.min(at, rest.length), 0, { ...m })
    setModules(rest)
    selected = moduleKey(m)
  }
  function remove(i: number) {
    if (i < 0) return
    if (moduleKey(modules[i]) === selected) selected = null
    setModules(modules.filter((_, k) => k !== i))
  }
  function move(i: number, d: -1 | 1) {
    const j = i + d
    if (j < 0 || j >= modules.length) return
    const list = [...modules]
    ;[list[i], list[j]] = [list[j], list[i]]
    setModules(list)
  }
  function patch(p: Partial<PanelModule>) {
    if (selIndex < 0) return
    setModules(modules.map((m, i) => (i === selIndex ? { ...m, ...p } : m)))
  }

  // pointer dragging: HTML drag and drop does not reach the page on Windows (Tauri takes file drops).
  // While over the panel the dragged module takes its place in a live preview and the others
  // slide aside; the order is saved only on release.
  type Drag = { from: 'panel' | 'gallery'; m: PanelModule; x: number; y: number; dx: number; dy: number; over: 'panel' | 'gallery' | null; index: number }
  let drag = $state<Drag | null>(null)
  let dragged = false
  let settleUntil = 0
  // the tile the last move went past: swapping straight back with it would make the two flip-flop
  let passed: string | null = null
  const shown = $derived.by(() => {
    if (!drag || drag.over !== 'panel' || drag.index < 0) return modules
    const k = moduleKey(drag.m)
    const rest = modules.filter((x) => moduleKey(x) !== k)
    rest.splice(Math.min(drag.index, rest.length), 0, modules.find((x) => moduleKey(x) === k) ?? drag.m)
    return rest
  })
  function retarget(d: Drag) {
    const hit = document.elementFromPoint(d.x, d.y)
    const zone = hit?.closest<HTMLElement>('[data-zone]')?.dataset.zone
    if (zone === 'gallery') return void (d.over = 'gallery')
    if (zone !== 'panel') return void (d.over = null)
    d.over = 'panel'
    // the grid is still sliding from the last move: wait, or the tiles would swap back and forth
    if (performance.now() < settleUntil) return
    const tile = hit?.closest<HTMLElement>('[data-tile]')
    const under = tile?.dataset.key ?? null
    if (under !== passed) passed = null
    if (under && under !== moduleKey(d.m) && under !== passed) {
      const j = shown.findIndex((x) => moduleKey(x) === under)
      if (j >= 0 && j !== d.index) {
        d.index = j
        passed = under
        settleUntil = performance.now() + 200
      }
    } else if (!tile && d.index < 0) {
      d.index = modules.length
    }
  }
  // keeps the page moving while the pointer rests near its top or bottom edge
  let scroller: HTMLElement | null = null
  let raf = 0
  function autoscroll() {
    if (!drag || !scroller) return
    const r = scroller.getBoundingClientRect()
    const edge = 56
    const v = drag.y < r.top + edge ? -(r.top + edge - drag.y) / 4 : drag.y > r.bottom - edge ? (drag.y - (r.bottom - edge)) / 4 : 0
    if (v) scroller.scrollTop += v
    // also settles the place once the grid stops sliding under a resting pointer
    retarget(drag)
    raf = requestAnimationFrame(autoscroll)
  }
  function grab(e: PointerEvent, from: Drag['from'], m: PanelModule) {
    if (e.button !== 0) return
    const [sx, sy] = [e.clientX, e.clientY]
    const box = (e.currentTarget as HTMLElement).closest<HTMLElement>('[data-tile]')?.getBoundingClientRect()
    const start = from === 'panel' ? modules.findIndex((x) => moduleKey(x) === moduleKey(m)) : -1
    const onMove = (ev: PointerEvent) => {
      if (!drag) {
        if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < 5) return
        const [w, h] = ghostSize(m)
        passed = null
        drag = { from, m, x: ev.clientX, y: ev.clientY, dx: box ? sx - box.left : w / 2, dy: box ? sy - box.top : h / 2, over: from, index: start }
        scroller = document.querySelector<HTMLElement>('.page')
        document.documentElement.classList.add('dragging-tile')
        raf = requestAnimationFrame(autoscroll)
      }
      drag.x = ev.clientX
      drag.y = ev.clientY
      retarget(drag)
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
      cancelAnimationFrame(raf)
      document.documentElement.classList.remove('dragging-tile')
      if (!drag) return
      // the click that ends a drag must not also select or add
      dragged = true
      setTimeout(() => (dragged = false))
      drop(drag)
      drag = null
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }
  function drop(d: Drag) {
    if (d.over === 'gallery' && d.from === 'panel') return remove(modules.findIndex((x) => moduleKey(x) === moduleKey(d.m)))
    if (d.over !== 'panel' || d.index < 0) return
    if (shown.length > MAX) return
    setModules(shown.map((x) => ({ ...x })))
    selected = moduleKey(d.m)
  }
  const ghostSize = (m: PanelModule): [number, number] => ({ s: [64, 64], w: [136, 64], m: [136, 136], l: [280, 64] })[m.size] as [number, number]
  function keys(e: KeyboardEvent, i: number) {
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault()
      remove(i)
    } else if (e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowUp')) {
      e.preventDefault()
      move(i, -1)
    } else if (e.altKey && (e.key === 'ArrowRight' || e.key === 'ArrowDown')) {
      e.preventDefault()
      move(i, 1)
    } else if (e.key === 'Escape') {
      selected = null
    }
  }
  const editApi = $derived({
    selected,
    placeholder: drag?.over === 'panel' ? moduleKey(drag.m) : null,
    lifted: drag && drag.from === 'panel' && drag.over !== 'panel' ? moduleKey(drag.m) : null,
    name: modName,
    onselect: (i: number) => {
      if (dragged) return
      const k = moduleKey(modules[i])
      selected = selected === k ? null : k
    },
    onremove: remove,
    ongrab: (e: PointerEvent, i: number) => grab(e, 'panel', modules[i]),
    onkey: keys,
  })

  // the panel's own used/remaining choice
  let panelValue = $state(untrack(() => app.settings!.panel.value))
  $effect(() => {
    const v = app.settings?.panel.value
    if (v && !pending.has(PANEL)) untrack(() => (panelValue = v))
  })
  function setValue(v: typeof panelValue) {
    panelValue = v
    later(PANEL)
  }
  const panelMode = $derived<'used' | 'remaining'>(panelValue === 'app' ? (app.settings?.limit_display === 'remaining' ? 'remaining' : 'used') : panelValue)
  const previewSettings = $derived(app.settings ? { ...app.settings, panel: { modules, value: panelValue } } : null)

  // the desktop around the preview
  const dark = () => document.documentElement.dataset.theme === 'dark' || (document.documentElement.dataset.theme !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches)
  let backdrop: 'light' | 'dark' = $state(dark() ? 'dark' : 'light')
  const clock = $derived(fmtClock(now))
  const showsIcon = $derived(!(mac && tray.hide_icon && preview?.title))
  const openHint = $derived(t(mac ? 'menubar.panel.open.mac' : windows ? 'menubar.panel.open.windows' : 'menubar.panel.open.linux'))

  // text next to the icon
  const textCatalog: TrayItem[] = [
    ...(['limit', 'forecast'] as const).flatMap((kind) => POOLS.flatMap(([provider, pool]) => ['five_hour', 'seven_day'].map((window) => ({ kind, provider, pool, window, period: '' })))),
    ...(['tokens', 'cost', 'sessions', 'cache'] as const).flatMap((kind) => ['today', 'days7', 'month1'].map((period) => ({ kind, provider: '', pool: '', window: '', period }))),
    { kind: 'scan', provider: '', pool: '', window: '', period: '' },
  ]
  const itemKey = (it: TrayItem) => `${it.kind}:${it.provider}:${it.window}:${it.pool}:${it.period}`
  const itemName = (it: TrayItem) => {
    const lim = `${limitName(it.provider, it.pool)} · ${windowLabel(it.window)}`
    if (it.kind === 'limit') return lim
    if (it.kind === 'forecast') return t('menubar.item.forecast', { name: lim })
    if (it.kind === 'scan') return t('menubar.item.scan')
    return `${t(`menubar.item.${it.kind}`)} · ${t(`widget.period.${it.period}`)}`
  }
  const addable = $derived(textCatalog.filter((it) => !tray.items.some((x) => itemKey(x) === itemKey(it))))
  function moveItem(i: number, d: -1 | 1) {
    const j = i + d
    if (j < 0 || j >= tray.items.length) return
    const list = [...tray.items]
    ;[list[i], list[j]] = [list[j], list[i]]
    set('items', list)
  }
  const separators = [' · ', ' | ', ' / ', '  ']
  const sepChoice = $derived(separators.includes(tray.separator) ? tray.separator : 'custom')
  let customSep = $state(false)

  const iconLimits = [
    ['anthropic', 'five_hour'],
    ['anthropic', 'seven_day'],
    ['openai', 'five_hour'],
    ['openai', 'seven_day'],
    ['google', 'five_hour'],
    ['google', 'seven_day'],
  ]
</script>

{#snippet modGlyph(m: PanelModule)}
  {#if m.kind === 'provider' || m.kind === 'limit'}
    <BrandIcon provider={m.provider as Provider} size={16} always />
  {:else if m.kind === 'forecast'}
    <Icon name="clock" size={16} />
  {:else if m.kind === 'usage'}
    <Icon name="chart" size={16} />
  {:else if m.kind === 'scan'}
    <Icon name="refresh" size={16} />
  {:else}
    <Icon name={actionIcon[m.action]} size={16} />
  {/if}
{/snippet}

<header class="head">
  <h1>{t('menubar.title')}</h1>
  <p class="subtle">{t('menubar.subtitle')}</p>
</header>

<section class="card studio">
  <div class="sh">
    <div>
      <h2>{t('menubar.panel')}</h2>
      <p class="subtle small">{openHint}</p>
    </div>
    <span class="spacer"></span>
    <div class="controls">
      <label class="ctl">
        <span class="subtle small">{t('menubar.value')}</span>
        <Segmented label={t('menubar.value')} value={panelValue} options={(['app', 'used', 'remaining'] as const).map((v) => ({ value: v, label: t(`menubar.value.${v}`) }))} onchange={setValue} />
      </label>
      <label class="ctl">
        <span class="subtle small">{t('menubar.backdrop')}</span>
        <Segmented label={t('menubar.backdrop')} bind:value={backdrop} options={[{ value: 'light', label: t('settings.theme.light') }, { value: 'dark', label: t('settings.theme.dark') }]} />
      </label>
    </div>
  </div>

  <div class="workbench">
    <div class="stage {backdrop}" data-theme={backdrop}>
      <div class="menubar" aria-hidden="true">
        <span class="spacer"></span>
        <span class="tray">
          {#if showsIcon}<img src="/app-icon.png" alt="" width="16" height="16" />{/if}
          {#if !windows && preview?.title}<span class="title">{preview.title}</span>{/if}
        </span>
        <span class="clock">{clock}</span>
      </div>
      <div class="glass">
        {#if previewSettings}
          {#if shown.length}
            <PanelGrid modules={shown} {data} settings={previewSettings} {now} edit={editApi} />
          {:else}
            <p class="empty" class:target={drag?.over === 'panel'} data-zone="panel">{t('menubar.panel.empty')}</p>
          {/if}
        {/if}
        <div class="foot" aria-hidden="true">
          <span>{t('widget.updated', { t: clock })}</span>
          <span class="pill">{t('panel.edit')}</span>
        </div>
      </div>
    </div>

    <aside class="side">
      <div class="inspector" aria-live="polite">
        {#if sel}
          <div class="ih">
            <span class="disc">{@render modGlyph(sel)}</span>
            <div class="grow">
              <strong>{modName(sel)}</strong>
              <div class="subtle small">{t(`menubar.mod.desc.${sel.kind}`)}</div>
            </div>
          </div>
          {#if SIZES[sel.kind].length > 1}
            <div class="field-row">
              <span>{t('menubar.panel.size')}</span>
              <div class="sizes" role="radiogroup" aria-label={t('menubar.panel.size')}>
                {#each ORDER.filter((z) => SIZES[sel.kind].includes(z)) as z (z)}
                  <button class="size" role="radio" aria-checked={sel.size === z} class:on={sel.size === z} onclick={() => patch({ size: z })}>
                    <span class="shape {z}" aria-hidden="true"></span>
                    <span class="small">{t(`menubar.panel.size.${z}`)}</span>
                  </button>
                {/each}
              </div>
            </div>
          {/if}
          {#if sel.kind === 'limit' || sel.kind === 'provider'}
            <div class="field-row">
              <span>{t('menubar.panel.style')}</span>
              <Segmented label={t('menubar.panel.style')} value={sel.style || 'ring'} options={[{ value: 'ring', label: t('menubar.panel.style.ring') }, { value: 'bar', label: t('menubar.panel.style.bar') }]} onchange={(v) => patch({ style: v })} />
            </div>
          {/if}
          <div class="actions">
            <button class="btn ghost small" disabled={selIndex === 0} onclick={() => move(selIndex, -1)}><Icon name="up" size={13} />{t('menubar.panel.earlier')}</button>
            <button class="btn ghost small" disabled={selIndex === modules.length - 1} onclick={() => move(selIndex, 1)}><Icon name="down" size={13} />{t('menubar.panel.later')}</button>
            <span class="spacer"></span>
            <button class="btn small danger" onclick={() => remove(selIndex)}>{t('menubar.panel.removeShort')}</button>
          </div>
        {:else}
          <p class="subtle small hint">{t('menubar.panel.select')}</p>
        {/if}
      </div>

      <div class="gallery" class:target={drag?.from === 'panel' && drag.over === 'gallery'} data-zone="gallery">
        <div class="gh">
          <h3>{t('menubar.gallery')}</h3>
          <span class="subtle small">{t('menubar.panel.count', { n: modules.length, max: MAX })}</span>
        </div>
        <p class="subtle small">{drag?.from === 'panel' ? t('menubar.gallery.drop') : t('menubar.gallery.help')}</p>
        <label class="search">
          <Icon name="search" size={14} />
          <input class="field" type="search" placeholder={t('menubar.gallery.search')} aria-label={t('menubar.gallery.search')} bind:value={query} />
        </label>
        <div class="cats" role="radiogroup" aria-label={t('menubar.gallery')}>
          {#each ['all', 'limits', 'forecast', 'usage', 'controls'] as c (c)}
            <button class="cat" role="radio" aria-checked={cat === c} class:on={cat === c} onclick={() => (cat = c)}>{t(`menubar.cat.${c}`)}</button>
          {/each}
        </div>
        <div class="list">
          {#each groups as g (g.c)}
            <h4>{t(`menubar.cat.${g.c}`)}</h4>
            <ul role="list">
              {#each g.items as m (moduleKey(m))}
                {@const inside = has(m)}
                <li>
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div class="gi" class:inside onpointerdown={(e) => !inside && grab(e, 'gallery', m)}>
                    <span class="disc">{@render modGlyph(m)}</span>
                    <span class="gt">
                      <span class="gn">{modName(m)}</span>
                      <span class="subtle small">{inside ? t('menubar.panel.added') : t(`menubar.mod.desc.${m.kind}`)}</span>
                    </span>
                    {#if inside}
                      <button class="round on" aria-label={t('menubar.panel.remove', { name: modName(m) })} title={t('menubar.panel.remove', { name: modName(m) })} onclick={() => remove(modules.findIndex((x) => moduleKey(x) === moduleKey(m)))}><Icon name="check" size={12} /></button>
                    {:else}
                      <button class="round" aria-label={t('menubar.panel.add', { name: modName(m) })} title={t('menubar.panel.add', { name: modName(m) })} disabled={modules.length >= MAX} onclick={() => !dragged && add(m)}><Icon name="plus" size={12} /></button>
                    {/if}
                  </div>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="subtle small">{t('menubar.gallery.none')}</p>
          {/each}
        </div>
        {#if modules.length >= MAX}<p class="subtle small">{t('menubar.panel.full')}</p>{/if}
      </div>
      <button
        class="btn ghost small reset"
        onclick={() => {
          selected = null
          setModules(defaults())
        }}><Icon name="refresh" size={13} />{t('menubar.panel.default')}</button
      >
    </aside>
  </div>
  <p class="subtle small help">{t('menubar.panel.help')}</p>
</section>

<div class="lower">
  <section class="card group">
    <h2>{t('menubar.text')}</h2>
    {#if windows}
      <p class="subtle small">{t('menubar.text.windows')}</p>
    {:else}
      <p class="subtle small">{t('menubar.text.help')}</p>
      <div class="item">
        <span>{t('menubar.text.show')}</span>
        <Toggle checked={tray.title} label={t('menubar.text.show')} onchange={(v) => set('title', v)} />
      </div>
      <div class="textbody" class:off={!tray.title}>
        <div class="item col">
          <span>{t('menubar.text.items')}</span>
          {#if tray.items.length}
            <ul class="chips" role="list">
              {#each tray.items as it, i (itemKey(it))}
                <li class="chip">
                  <span class="cn">{itemName(it)}</span>
                  <span class="cv">{preview?.parts[i] || '—'}</span>
                  <button class="cb" aria-label="{itemName(it)}: {t('menubar.panel.earlier')}" title={t('menubar.panel.earlier')} disabled={i === 0} onclick={() => moveItem(i, -1)}><Icon name="up" size={11} /></button>
                  <button class="cb" aria-label="{itemName(it)}: {t('menubar.panel.later')}" title={t('menubar.panel.later')} disabled={i === tray.items.length - 1} onclick={() => moveItem(i, 1)}><Icon name="down" size={11} /></button>
                  <button class="cb x" aria-label={t('menubar.panel.remove', { name: itemName(it) })} title={t('menubar.panel.remove', { name: itemName(it) })} onclick={() => set('items', tray.items.filter((_, k) => k !== i))}><Icon name="close" size={10} /></button>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="subtle small">{t('menubar.text.empty')}</p>
          {/if}
          {#if addable.length && tray.items.length < 12}
            <div>
              <Select label={t('menubar.text.add')} value="" minWidth={260} options={[{ value: '', label: t('menubar.text.add') }, ...addable.map((it) => ({ value: itemKey(it), label: itemName(it) }))]} onchange={(v) => v && set('items', [...tray.items, addable.find((it) => itemKey(it) === v)!])} />
            </div>
          {/if}
          {#if tray.title && tray.items.length && preview && !preview.title}<p class="subtle small">{t('menubar.text.none')}</p>{/if}
        </div>
        <div class="item">
          <span>{t('menubar.label')}</span>
          <Segmented label={t('menubar.label')} value={tray.label} options={(['name', 'short', 'none'] as const).map((v) => ({ value: v, label: t(`menubar.label.${v}`) }))} onchange={(v) => set('label', v)} />
        </div>
        <div class="item">
          <span>{t('menubar.value')}</span>
          <Segmented label={t('menubar.value')} value={tray.value} options={(['app', 'used', 'remaining'] as const).map((v) => ({ value: v, label: t(`menubar.value.${v}`) }))} onchange={(v) => set('value', v)} />
        </div>
        <div class="item">
          <span>{t('menubar.style')}</span>
          <Segmented label={t('menubar.style')} value={tray.style} options={(['percent', 'bar', 'bar_percent', 'dot'] as const).map((v) => ({ value: v, label: t(`menubar.style.${v}`) }))} onchange={(v) => set('style', v)} />
        </div>
        <div class="item">
          <span>{t('menubar.reset')}</span>
          <Segmented label={t('menubar.reset')} value={tray.reset} options={(['none', 'countdown', 'time'] as const).map((v) => ({ value: v, label: t(`menubar.reset.${v}`) }))} onchange={(v) => set('reset', v)} />
        </div>
        <div class="item">
          <span>{t('menubar.separator')}</span>
          <div class="inline">
            <Segmented
              label={t('menubar.separator')}
              value={customSep ? 'custom' : sepChoice}
              options={[...separators.map((v) => ({ value: v, label: v.trim() || t('menubar.separator.space') })), { value: 'custom', label: t('menubar.separator.custom') }]}
              onchange={(v) => {
                customSep = v === 'custom'
                if (v !== 'custom') set('separator', v)
              }}
            />
            {#if customSep || sepChoice === 'custom'}
              <input class="field tiny" type="text" maxlength="6" value={tray.separator} aria-label={t('menubar.separator.custom')} oninput={(e) => e.currentTarget.value && set('separator', e.currentTarget.value)} />
            {/if}
          </div>
        </div>
        <div class="item">
          <span>{t('menubar.ends')}</span>
          <div class="inline">
            <input class="field affix" type="text" maxlength="12" value={tray.prefix} placeholder={t('menubar.prefix')} aria-label={t('menubar.prefix')} oninput={(e) => set('prefix', e.currentTarget.value)} />
            <input class="field affix" type="text" maxlength="12" value={tray.suffix} placeholder={t('menubar.suffix')} aria-label={t('menubar.suffix')} oninput={(e) => set('suffix', e.currentTarget.value)} />
          </div>
        </div>
        <div class="item">
          <div><span>{t('menubar.warn')}</span><div class="subtle small">{t('menubar.warn.help', { warn: fmtPct(app.settings?.widget.warn_at ?? 80) })}</div></div>
          <Toggle checked={tray.warn_mark} label={t('menubar.warn')} onchange={(v) => set('warn_mark', v)} />
        </div>
        {#if mac}
          <div class="item">
            <div><span>{t('menubar.hideIcon')}</span><div class="subtle small">{t('menubar.hideIcon.help')}</div></div>
            <Toggle checked={tray.hide_icon} label={t('menubar.hideIcon')} onchange={(v) => set('hide_icon', v)} />
          </div>
        {/if}
      </div>
    {/if}
  </section>

  <section class="card group">
    <h2>{t('menubar.icon')}</h2>
    {#if windows}<p class="subtle small">{t('settings.tray.help')}</p>{/if}
    <div class="item">
      <div><span>{t('settings.tray.show')}</span><div class="subtle small">{t('settings.tray.showHelp', { warn: fmtPct(app.settings?.widget.warn_at ?? 80), high: fmtPct(app.settings?.widget.high_at ?? 95) })}</div></div>
      <Toggle checked={tray.show_percent} label={t('settings.tray.show')} onchange={(v) => set('show_percent', v)} />
    </div>
    <div class="item">
      <span>{t('settings.tray.limit')}</span>
      <Select
        label={t('settings.tray.limit')}
        value={tray.limit}
        disabled={!tray.show_percent}
        minWidth={220}
        options={[{ value: 'auto', label: t('settings.tray.auto') }, ...iconLimits.map(([p, w]) => ({ value: `${p}:${w}`, label: `${t(`provider.${p}`)} · ${windowLabel(w)}` }))]}
        onchange={(v) => set('limit', v)}
      />
    </div>
  </section>
</div>

{#if drag && previewSettings}
  {@const [w, h] = ghostSize(drag.m)}
  <div class="ghost-tile" data-theme={backdrop} style:left="{drag.x - drag.dx}px" style:top="{drag.y - drag.dy}px" style:width="{w}px" style:height="{h}px" aria-hidden="true">
    <PanelModuleView m={drag.m} {data} settings={previewSettings} {now} mode={panelMode} edit />
  </div>
{/if}

<style>
  .head {
    padding: 16px 0 14px;
  }
  .head p {
    margin-top: 4px;
  }
  .spacer,
  .grow {
    flex: 1;
  }
  .sh,
  .gh,
  .ih,
  .actions,
  .inline {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .sh {
    flex-wrap: wrap;
    margin-bottom: 14px;
  }
  .sh h2 {
    margin-bottom: 2px;
  }
  .studio .help {
    margin-top: 12px;
  }

  .workbench {
    display: grid;
    grid-template-columns: minmax(380px, 1fr) minmax(300px, 380px);
    gap: 18px;
    align-items: start;
  }
  @container main (max-width: 900px) {
    .workbench {
      grid-template-columns: 1fr;
    }
  }

  /* the desktop the panel opens over: a wallpaper and a menu bar */
  .stage {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    min-width: 0;
    padding-bottom: 32px;
    border-radius: 16px;
    overflow: hidden;
    color: var(--ink);
    isolation: isolate;
  }
  .stage.light {
    background:
      radial-gradient(120% 90% at 10% 100%, #f6c7a8 0%, transparent 55%),
      radial-gradient(90% 80% at 100% 0%, #a9c6f5 0%, transparent 60%),
      linear-gradient(160deg, #e8ecf6, #f4e9e2);
  }
  .stage.dark {
    background:
      radial-gradient(120% 90% at 0% 100%, #5b3a6e 0%, transparent 55%),
      radial-gradient(90% 80% at 100% 0%, #1f4f8a 0%, transparent 60%),
      linear-gradient(160deg, #141a2b, #0d0f15);
  }
  .menubar {
    display: flex;
    align-items: center;
    gap: 14px;
    align-self: stretch;
    min-height: 28px;
    padding: 4px 14px;
    background: color-mix(in srgb, var(--surface) 55%, transparent);
    backdrop-filter: blur(24px) saturate(1.6);
    -webkit-backdrop-filter: blur(24px) saturate(1.6);
    font: 500 13px/1.3 -apple-system, 'SF Pro Text', var(--font);
  }
  .tray {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 2px 6px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--ink) 12%, transparent);
  }
  .title {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .clock {
    flex: none;
    font-variant-numeric: tabular-nums;
  }
  .glass {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 304px;
    max-width: calc(100% - 28px);
    margin: 8px 14px 0 0;
    padding: 12px;
    border-radius: 18px;
    border: 0.5px solid var(--cc-edge);
    background: color-mix(in srgb, var(--surface) 46%, transparent);
    backdrop-filter: blur(30px) saturate(1.8);
    -webkit-backdrop-filter: blur(30px) saturate(1.8);
    box-shadow: 0 18px 50px rgba(0, 0, 0, 0.28), 0 0 0 0.5px rgba(0, 0, 0, 0.12);
  }
  .empty.target {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .empty {
    margin: 0;
    padding: 40px 18px;
    border-radius: 18px;
    border: 1.5px dashed var(--cc-track);
    color: var(--ink-2);
    font-size: 12.5px;
    text-align: center;
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2px;
    font-size: 11px;
    color: var(--ink-3);
  }
  .pill {
    padding: 4px 12px;
    border-radius: 999px;
    background: var(--cc-tile);
    color: var(--ink);
    font-size: 12px;
    font-weight: 600;
  }

  .side {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  .inspector {
    padding: 14px;
    border-radius: 14px;
    border: 0.5px solid var(--hairline-strong);
    background: var(--surface-2);
  }
  .hint {
    margin: 0;
    padding: 8px 0;
    text-align: center;
  }
  .ih {
    align-items: flex-start;
    padding-bottom: 12px;
    border-bottom: 0.5px solid var(--hairline);
  }
  .ih strong {
    display: block;
    font-size: 13.5px;
    line-height: 1.3;
  }
  .disc {
    display: grid;
    place-items: center;
    flex: none;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    background: var(--surface-press);
    color: var(--ink);
  }
  .field-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 12px;
    padding: 12px 0 0;
  }
  .sizes {
    display: flex;
    gap: 6px;
  }
  .size {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: space-between;
    gap: 5px;
    min-width: 62px;
    min-height: 62px;
    padding: 8px 6px 6px;
    border: 0.5px solid var(--hairline-strong);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink-2);
    transition: border-color var(--dur) var(--ease), background var(--dur) var(--ease);
  }
  .size:hover {
    background: var(--surface-hover);
  }
  .size.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
    color: var(--ink);
  }
  .size:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .shape {
    display: block;
    margin: auto 0;
    border-radius: 4px;
    background: color-mix(in srgb, var(--ink) 22%, transparent);
  }
  .size.on .shape {
    background: var(--accent);
  }
  .shape.s {
    width: 12px;
    height: 12px;
    border-radius: 50%;
  }
  .shape.w {
    width: 26px;
    height: 12px;
  }
  .shape.m {
    width: 26px;
    height: 26px;
  }
  .shape.l {
    width: 48px;
    height: 12px;
  }
  .actions {
    padding-top: 14px;
  }
  .btn.danger {
    color: var(--bad-ink);
  }

  .gallery {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px;
    border-radius: 14px;
    border: 1px dashed transparent;
    background: var(--surface-2);
    transition: border-color var(--dur) var(--ease), background var(--dur) var(--ease);
  }
  .gallery.target {
    border-color: var(--critical);
    background: color-mix(in srgb, var(--critical) 8%, var(--surface-2));
  }
  .gh h3 {
    margin: 0;
    flex: 1;
  }
  .gallery > p {
    margin: 0;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--ink-3);
  }
  .search .field {
    flex: 1;
    min-width: 0;
  }
  .cats {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .cat {
    height: 26px;
    padding: 0 11px;
    border: 0.5px solid var(--hairline-strong);
    border-radius: 999px;
    background: var(--surface);
    color: var(--ink-2);
    font-size: 12px;
    font-weight: 500;
    transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
  }
  .cat:hover {
    background: var(--surface-hover);
    color: var(--ink);
  }
  .cat.on {
    border-color: transparent;
    background: var(--ink);
    color: var(--surface);
  }
  .cat:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .list {
    max-height: 430px;
    overflow-y: auto;
    margin: 0 -6px;
    padding: 0 6px;
  }
  h4 {
    margin: 12px 0 6px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--ink-3);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .gi {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 8px;
    border-radius: 10px;
    cursor: grab;
    touch-action: none;
    user-select: none;
    transition: background var(--dur) var(--ease);
  }
  .gi:hover {
    background: var(--surface-hover);
  }
  .gi.inside {
    cursor: default;
  }
  .gt {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
  }
  .gn {
    font-size: 12.5px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .round {
    display: grid;
    place-items: center;
    flex: none;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0.5px solid var(--hairline-strong);
    border-radius: 50%;
    background: var(--surface);
    color: var(--ink-2);
  }
  .round:hover:not(:disabled) {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: transparent;
  }
  .round.on {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: transparent;
  }
  .round.on:hover {
    background: var(--critical);
  }
  .round:disabled {
    opacity: 0.4;
  }
  .round:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .reset {
    align-self: flex-start;
  }

  .lower {
    display: flex;
    flex-direction: column;
    gap: 16px;
    margin-top: 16px;
  }
  .group h2 {
    margin-bottom: 4px;
  }
  .item {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 16px;
    padding: 10px 0;
    border-bottom: 0.5px solid var(--hairline);
    min-height: 44px;
  }
  .item:last-child {
    border-bottom: 0;
  }
  .item > div:first-child:not(:last-child) {
    flex: 1 1 180px;
    min-width: 0;
  }
  .item.col {
    flex-direction: column;
    align-items: stretch;
  }
  .textbody {
    transition: opacity var(--dur) var(--ease);
  }
  .textbody.off {
    opacity: 0.5;
  }
  .chips {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 6px 5px 12px;
    border-radius: 10px;
    background: var(--surface-2);
    border: 0.5px solid var(--hairline);
  }
  .cn {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    font-weight: 600;
  }
  .cv {
    font-size: 12px;
    color: var(--ink-2);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .cb {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-2);
  }
  .cb:hover:not(:disabled) {
    background: var(--surface-hover);
  }
  .cb.x:hover {
    color: var(--bad-ink);
  }
  .cb:disabled {
    opacity: 0.35;
  }
  .field.tiny {
    width: 70px;
  }
  .field.affix {
    width: 110px;
  }

  /* the tile itself follows the pointer, lifted */
  .ghost-tile {
    position: fixed;
    z-index: 50;
    border-radius: 16px;
    background: transparent;
    color: var(--ink);
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.3);
    pointer-events: none;
    transform: scale(1.04);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
  }
  .ghost-tile :global(.mod) {
    background: color-mix(in srgb, var(--surface) 82%, transparent);
  }
  :global(html.dragging-tile),
  :global(html.dragging-tile *) {
    cursor: grabbing !important;
    user-select: none !important;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 10px 16px;
  }
  .ctl {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  @media (prefers-reduced-motion: reduce) {
    .gallery,
    .gi,
    .size,
    .textbody {
      transition: none;
    }
  }
</style>
