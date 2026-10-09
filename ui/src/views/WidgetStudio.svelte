<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte'
  import { app, saveSettings } from '../lib/store.svelte'
  import { api, PROVIDERS, type Provider, type WidgetData, type WidgetItemKind, type WidgetSettings } from '../lib/api'
  import { fmtPct, searchKey, t } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import Toggle from '../components/Toggle.svelte'
  import ColorPick from '../components/ColorPick.svelte'
  import Icon from '../components/Icon.svelte'
  import WidgetView from '../components/WidgetView.svelte'

  // only Windows lets the app see a full-screen program; macOS always hides the widget then
  const behaviorKeys = $derived(
    app.info?.platform === 'windows' ? (['visible', 'always_on_top', 'lock_position', 'auto_hide_fullscreen'] as const) : (['visible', 'always_on_top', 'lock_position'] as const),
  )

  let ws = $state<WidgetSettings>(structuredClone($state.snapshot(app.settings!.widget)) as WidgetSettings)
  let data = $state<WidgetData | null>(null)
  let backdrop: 'light' | 'dark' | 'photo' = $state('photo')
  // a wide widget (single line, large scale) is shrunk to fit the preview instead of being cut off
  let stageW = $state(0)
  let natW = $state(0)
  let natH = $state(0)
  const fit = $derived(natW && stageW ? Math.min(1, (stageW - 56) / natW) : 1)

  let fonts = $state<string[]>([])
  let fontQuery = $state('')
  onMount(() => {
    api.widgetData().then((d) => (data = d))
    api.listFonts().then((f) => (fonts = f)).catch(() => {})
  })
  const SUGGESTED = ['Segoe UI Variable Display', 'Segoe UI', 'Aptos', 'Bahnschrift', 'Calibri', 'Cascadia Mono', 'Consolas', 'Georgia', 'Verdana', 'Arial']
  const suggested = $derived(SUGGESTED.filter((f) => fonts.includes(f)))
  const fontMatches = $derived.by(() => {
    const q = searchKey(fontQuery.trim())
    return q ? fonts.filter((f) => searchKey(f).includes(q)) : fonts
  })

  // only fields edited here are saved, so a stale copy never overwrites the tray's or shortcut's changes
  type Key = keyof WidgetSettings
  const pending = new Map<Key, number>()
  let edits = 0
  function sync(saved: WidgetSettings) {
    for (const k of Object.keys(saved) as Key[]) if (!pending.has(k)) (ws as Record<Key, unknown>)[k] = structuredClone(saved[k])
  }
  $effect(() => {
    const saved = app.settings?.widget
    if (saved) {
      const snap = $state.snapshot(saved) as WidgetSettings
      untrack(() => sync(snap))
    }
  })

  let timer: ReturnType<typeof setTimeout> | undefined
  function flush() {
    clearTimeout(timer)
    timer = undefined
    if (!pending.size) return
    const marks = [...pending]
    const patch = Object.fromEntries(marks.map(([k]) => [k, $state.snapshot(ws[k])])) as Partial<WidgetSettings>
    saveSettings((c) => ({ widget: { ...c.widget, ...patch } }))
      .catch(() => {})
      .finally(() => {
        for (const [k, n] of marks) if (pending.get(k) === n) pending.delete(k)
        if (app.settings) sync($state.snapshot(app.settings.widget) as WidgetSettings)
      })
  }
  function commit(...keys: Key[]) {
    for (const k of keys) pending.set(k, ++edits)
    clearTimeout(timer)
    timer = setTimeout(flush, 250)
  }
  onDestroy(flush)
  function set<K extends Key>(k: K, v: WidgetSettings[K]) {
    ws[k] = v
    commit(k)
  }
  function move(i: number, d: -1 | 1) {
    const j = i + d
    if (j < 0 || j >= ws.items.length) return
    const items = [...ws.items]
    ;[items[i], items[j]] = [items[j], items[i]]
    ws.items = items
    commit('items')
  }
  function toggleItem(i: number, v: boolean) {
    ws.items[i].enabled = v
    commit('items')
  }
  function toggleProvider(p: Provider, v: boolean) {
    const all = (data?.providers ?? PROVIDERS) as Provider[]
    const cur = ws.providers.length ? [...ws.providers] : [...all]
    const next = v ? [...new Set([...cur, p])] : cur.filter((x) => x !== p)
    // an empty list means "all", so the last provider cannot be switched off
    if (!next.length) return
    ws.providers = next.length === all.length ? [] : next
    commit('providers')
  }
  const providerOn = (p: Provider) => ws.providers.length === 0 || ws.providers.includes(p)
  const lastOn = (p: Provider) => {
    const all = (data?.providers ?? providers) as Provider[]
    return all.includes(p) && providerOn(p) && all.filter(providerOn).length === 1
  }
  async function reset() {
    const d: WidgetSettings = {
      visible: ws.visible, opacity: 0.85, size: 'm', scale: 1, x: ws.x, y: ws.y, anchor: 'bottom-right', auto_hide_fullscreen: true,
      layout: 'horizontal',
      items: (['primary', 'cost', 'limit_five_hour', 'limit_seven_day', 'tools', 'week_tokens', 'week_cost', 'month_cost', 'updated'] as WidgetItemKind[]).map((k, i) => ({ kind: k, enabled: i < 4 })),
      providers: [], primary_period: 'today', primary_metric: 'tokens', limit_style: 'ring', theme: 'system', accent: '',
      corner_radius: 14, border: true, shadow: false, show_labels: true, show_icons: false, show_reset_time: false, warn_at: 70, high_at: 90,
      always_on_top: true, lock_position: false, click_action: 'open_dashboard',
      font_family: '', text_scale: 1, number_scale: 1, number_weight: 700, tabular_nums: true,
      // the shortcut changes only in Settings (it must register first)
      hotkey: ws.hotkey,
    }
    ws = d
    commit(...(Object.keys(d) as Key[]).filter((k) => k !== 'visible' && k !== 'x' && k !== 'y' && k !== 'anchor' && k !== 'hotkey'))
    api.placeWidget('bottom-right')
  }
  const providers: Provider[] = PROVIDERS
</script>

<header class="bar">
  <h1>{t('ws.title')}</h1>
  <span class="spacer"></span>
  <button class="btn" onclick={reset}><Icon name="refresh" size={14} />{t('ws.reset')}</button>
</header>

<div class="studio">
  <section class="preview card">
    <div class="ph">
      <h2>{t('ws.preview')}</h2>
      <span class="spacer"></span>
      <Segmented label={t('ws.backdrop')} bind:value={backdrop} options={[{ value: 'photo', label: t('ws.backdrop.photo') }, { value: 'light', label: t('settings.theme.light') }, { value: 'dark', label: t('settings.theme.dark') }]} />
    </div>
    <div class="stage {backdrop}" data-theme={ws.theme === 'system' ? undefined : ws.theme} bind:clientWidth={stageW}>
      <div class="fit" style:width={natW ? `${natW * fit}px` : undefined} style:height={natH ? `${natH * fit}px` : undefined}>
        <div class="natural" bind:offsetWidth={natW} bind:offsetHeight={natH} style:transform="scale({fit})">
          <WidgetView {data} {ws} mode={app.settings?.limit_display ?? 'used'} colors={app.settings?.provider_colors} tintIcons={app.settings?.tint_icons} />
        </div>
      </div>
    </div>
    <p class="subtle small">{t('ws.preview.help')}</p>
  </section>

  <div class="panels">
    <section class="card group">
      <h2>{t('ws.content')}</h2>
      <ul class="items" aria-label={t('ws.content')}>
        {#each ws.items as it, i (it.kind)}
          <li class:off={!it.enabled}>
            <Toggle checked={it.enabled} label={t(`ws.item.${it.kind}`)} onchange={(v) => toggleItem(i, v)} />
            <span class="iname">{t(`ws.item.${it.kind}`)}</span>
            <span class="spacer"></span>
            <button class="btn ghost icon" aria-label={t('ws.up')} disabled={i === 0} onclick={() => move(i, -1)}><Icon name="up" size={14} /></button>
            <button class="btn ghost icon" aria-label={t('ws.down')} disabled={i === ws.items.length - 1} onclick={() => move(i, 1)}><Icon name="down" size={14} /></button>
          </li>
        {/each}
      </ul>
      <div class="item">
        <span>{t('ws.period')}</span>
        <Segmented label={t('ws.period')} value={ws.primary_period} options={[{ value: 'today', label: t('widget.period.today') }, { value: 'days7', label: t('widget.period.days7') }, { value: 'month1', label: t('widget.period.month1') }]} onchange={(v) => set('primary_period', v)} />
      </div>
      <div class="item">
        <span>{t('ws.metric')}</span>
        <Segmented label={t('ws.metric')} value={ws.primary_metric} options={[{ value: 'tokens', label: t('metric.tokens') }, { value: 'cost', label: t('metric.cost') }]} onchange={(v) => set('primary_metric', v)} />
      </div>
      <div class="item">
        <span>{t('limits.mode')}<span class="subtle small block">{t('limits.mode.help')}</span></span>
        <Segmented label={t('limits.mode')} value={app.settings?.limit_display ?? 'used'} options={[{ value: 'used', label: t('limits.mode.used') }, { value: 'remaining', label: t('limits.mode.remaining') }]} onchange={(v) => saveSettings({ limit_display: v })} />
      </div>
      <div class="item">
        <span>{t('ws.providers')}</span>
        <div class="row">
          {#each providers as p (p)}
            <label class="chk"><Toggle checked={providerOn(p)} disabled={lastOn(p)} label={t(`provider.${p}`)} onchange={(v) => toggleProvider(p, v)} />{t(`provider.${p}`)}</label>
          {/each}
        </div>
      </div>
    </section>

    <section class="card group">
      <h2>{t('ws.look')}</h2>
      <div class="item">
        <span>{t('ws.layout')}</span>
        <Segmented label={t('ws.layout')} value={ws.layout} options={[{ value: 'horizontal', label: t('ws.layout.horizontal') }, { value: 'vertical', label: t('ws.layout.vertical') }, { value: 'line', label: t('ws.layout.line') }]} onchange={(v) => set('layout', v)} />
      </div>
      <div class="item">
        <span>{t('ws.limitStyle')}</span>
        <Segmented label={t('ws.limitStyle')} value={ws.limit_style} options={[{ value: 'ring', label: t('ws.limitStyle.ring') }, { value: 'bar', label: t('ws.limitStyle.bar') }, { value: 'text', label: t('ws.limitStyle.text') }]} onchange={(v) => set('limit_style', v)} />
      </div>
      <div class="item">
        <span>{t('ws.theme')}</span>
        <Segmented label={t('ws.theme')} value={ws.theme} options={[{ value: 'system', label: t('ws.theme.app') }, { value: 'light', label: t('settings.theme.light') }, { value: 'dark', label: t('settings.theme.dark') }]} onchange={(v) => set('theme', v)} />
      </div>
      <div class="item">
        <span>{t('ws.accent')}</span>
        <ColorPick value={ws.accent} label={t('ws.accent')} defaultLabel={t('ws.accent.system')} customLabel={t('ws.accent.custom')} swatches={['#2a78d6', '#1baf7a', '#eb6834', '#e87ba4', '#4a3aa7', '#52514e']} onchange={(v) => set('accent', v)} />
      </div>
      {#each [['scale', 0.6, 2, 0.05, (v: number) => fmtPct(v * 100)], ['opacity', 0.3, 1, 0.05, (v: number) => fmtPct(v * 100)], ['corner_radius', 0, 28, 1, (v: number) => `${v}px`]] as const as [key, min, max, step, fmt] (key)}
        <div class="item">
          <span>{t(`ws.${key}`)}</span>
          <div class="row slider">
            <input type="range" {min} {max} {step} value={ws[key]} aria-label={t(`ws.${key}`)} oninput={(e) => set(key, Number(e.currentTarget.value))} />
            <span class="num val">{fmt(ws[key])}</span>
          </div>
        </div>
      {/each}
      {#each ['border', 'shadow', 'show_labels', 'show_icons', 'show_reset_time'] as const as key (key)}
        <div class="item">
          <span>{t(`ws.${key}`)}</span>
          <Toggle checked={ws[key]} label={t(`ws.${key}`)} onchange={(v) => set(key, v)} />
        </div>
      {/each}
    </section>

    <section class="card group">
      <h2>{t('ws.type')}</h2>
      <div class="item stack">
        <div class="fhead">
          <span>{t('ws.font')}</span>
          <span class="subtle small current" style="font-family:{ws.font_family ? `'${ws.font_family}', var(--font)` : 'var(--font)'}">{ws.font_family || t('ws.font.app')}</span>
        </div>
        <input type="search" class="field" placeholder={t('ws.font.search')} aria-label={t('ws.font.search')} bind:value={fontQuery} />
        <div class="fontlist" role="listbox" aria-label={t('ws.font')}>
          {#snippet fontOption(name: string, label: string)}
            <button role="option" aria-selected={ws.font_family === name} class:sel={ws.font_family === name} style="font-family:{name ? `'${name}', var(--font)` : 'var(--font)'}" onclick={() => set('font_family', name)}>
              <span>{label}</span>
              {#if ws.font_family === name}<Icon name="check" size={14} />{/if}
            </button>
          {/snippet}
          {#if !fontQuery.trim()}
            {@render fontOption('', t('ws.font.app'))}
            {#if suggested.length}
              <div class="fgh" role="presentation">{t('ws.font.suggested')}</div>
              {#each suggested as f (f)}{@render fontOption(f, f)}{/each}
            {/if}
            {#if fonts.length}<div class="fgh" role="presentation">{t('ws.font.all', { n: fonts.length })}</div>{/if}
          {/if}
          {#each fontMatches as f (f)}{@render fontOption(f, f)}{/each}
          {#if fontQuery.trim() && !fontMatches.length}<p class="subtle small none">{t('ws.font.none')}</p>{/if}
        </div>
      </div>
      {#each [['text_scale', 0.8, 1.6, 0.05], ['number_scale', 0.6, 2, 0.05]] as const as [key, min, max, step] (key)}
        <div class="item">
          <span>{t(`ws.${key}`)}</span>
          <div class="row slider">
            <input type="range" {min} {max} {step} value={ws[key]} aria-label={t(`ws.${key}`)} oninput={(e) => set(key, Number(e.currentTarget.value))} />
            <span class="num val">{fmtPct(ws[key] * 100)}</span>
          </div>
        </div>
      {/each}
      <div class="item">
        <span>{t('ws.number_weight')}</span>
        <div class="row slider">
          <input type="range" min="300" max="900" step="100" value={ws.number_weight} aria-label={t('ws.number_weight')} aria-valuetext={t(`ws.weight.${ws.number_weight}`)} oninput={(e) => set('number_weight', Number(e.currentTarget.value))} />
          <span class="val wname" style="font-weight:{ws.number_weight}">{t(`ws.weight.${ws.number_weight}`)}</span>
        </div>
      </div>
      <div class="item">
        <span>{t('ws.tabular_nums')}<span class="subtle small block">{t('ws.tabular_nums.help')}</span></span>
        <Toggle checked={ws.tabular_nums} label={t('ws.tabular_nums')} onchange={(v) => set('tabular_nums', v)} />
      </div>
    </section>

    <section class="card group">
      <h2>{t('ws.thresholds')}</h2>
      <p class="subtle small">{t('ws.thresholds.help')}</p>
      {#each [['warn_at', 'ws.warn'], ['high_at', 'ws.high']] as const as [key, label] (key)}
        <div class="item">
          <span><i class="dot {key}"></i>{t(label)}</span>
          <div class="row slider">
            <input type="range" min="1" max="100" step="1" value={ws[key]} aria-label={t(label)} oninput={(e) => set(key, Number(e.currentTarget.value))} />
            <span class="num val">{fmtPct(ws[key])}</span>
          </div>
        </div>
      {/each}
    </section>

    <section class="card group">
      <h2>{t('ws.behavior')}</h2>
      {#each behaviorKeys as key (key)}
        <div class="item">
          <span>{t(`ws.${key}`)}</span>
          <Toggle checked={ws[key]} label={t(`ws.${key}`)} onchange={(v) => set(key, v)} />
        </div>
      {/each}
      <div class="item">
        <span>{t('ws.click')}</span>
        <Segmented label={t('ws.click')} value={ws.click_action} options={[{ value: 'open_dashboard', label: t('ws.click.open') }, { value: 'none', label: t('ws.click.none') }]} onchange={(v) => set('click_action', v)} />
      </div>
      <div class="item">
        <span>{t('ws.position')}</span>
        <div class="corners" role="group" aria-label={t('ws.position')}>
          {#each [['top-left', '↖'], ['top-right', '↗'], ['bottom-left', '↙'], ['bottom-right', '↘']] as [c, g] (c)}
            <button class="btn" aria-label={t(`ws.corner.${c}`)} title={t(`ws.corner.${c}`)} onclick={() => { ws.anchor = c; api.placeWidget(c) }}>{g}</button>
          {/each}
        </div>
      </div>
    </section>
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 0 14px;
  }
  .spacer {
    flex: 1;
  }
  .studio {
    display: grid;
    grid-template-columns: minmax(320px, 1fr) minmax(360px, 1.1fr);
    gap: 16px;
    align-items: start;
  }
  @container main (max-width: 1040px) {
    .studio {
      grid-template-columns: 1fr;
    }
  }
  .preview {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .ph {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .stage {
    display: grid;
    place-items: center;
    min-height: 240px;
    border-radius: 12px;
    padding: 28px;
    overflow: hidden;
  }
  .natural {
    width: max-content;
    transform-origin: top left;
  }
  .stage.light {
    background: #eef0f3;
  }
  .stage.dark {
    background: #16181c;
  }
  .stage.photo {
    background:
      radial-gradient(120% 90% at 10% 10%, #7aa7e8 0%, transparent 55%),
      radial-gradient(100% 80% at 90% 20%, #f2a37b 0%, transparent 55%),
      radial-gradient(120% 100% at 50% 100%, #3d4f7a 0%, #1c2236 70%);
  }
  .panels {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .group h2 {
    margin-bottom: 6px;
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
  .items {
    list-style: none;
    margin: 4px 0 8px;
    padding: 0;
    border-radius: 10px;
    border: 0.5px solid var(--hairline);
    overflow: hidden;
  }
  .items li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px 6px 12px;
    border-bottom: 0.5px solid var(--hairline);
    background: var(--surface);
  }
  .items li:last-child {
    border-bottom: 0;
  }
  .items li.off .iname {
    color: var(--ink-3);
  }
  .icon {
    width: 28px;
    height: 28px;
    padding: 0;
    justify-content: center;
  }
  .chk {
    display: inline-flex;
    gap: 8px;
    align-items: center;
    margin-left: 12px;
  }
  .slider input {
    width: 170px;
    accent-color: var(--accent);
  }
  .val {
    min-width: 64px;
    text-align: right;
    color: var(--ink-2);
    font-size: 12.5px;
  }
  .item.stack {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
  .fhead {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
  }
  .current {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fontlist {
    max-height: 232px;
    overflow-y: auto;
    border: 0.5px solid var(--hairline);
    border-radius: 10px;
    padding: 4px;
    background: var(--surface);
  }
  .fontlist button {
    display: flex;
    width: 100%;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 7px 10px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--ink);
    font-size: 14px;
    text-align: left;
    cursor: default;
  }
  .fontlist button:hover {
    background: var(--surface-hover, var(--surface-press));
  }
  .fontlist button.sel {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--ink);
  }
  .fontlist button span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fgh {
    padding: 8px 10px 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--ink-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .none {
    padding: 8px 10px;
    margin: 0;
  }
  .wname {
    font-size: 13px;
  }
  .block {
    display: block;
    margin-top: 2px;
  }
  .dot {
    display: inline-block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    margin-right: 8px;
  }
  .dot.warn_at {
    background: var(--serious);
  }
  .dot.high_at {
    background: var(--critical);
  }
  .corners {
    display: grid;
    grid-template-columns: repeat(4, 34px);
    gap: 6px;
  }
  .corners .btn {
    padding: 0;
    justify-content: center;
    font-size: 15px;
  }
</style>
