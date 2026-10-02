<script lang="ts">
  // Widget customisation with a live preview. Edits apply to the preview at once and are
  // saved (and pushed to the real widget) shortly after.
  import { onMount } from 'svelte'
  import { app, saveSettings } from '../lib/store.svelte'
  import { api, type Provider, type WidgetData, type WidgetItemKind, type WidgetSettings } from '../lib/api'
  import { fmtPct, t } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Icon from '../components/Icon.svelte'
  import WidgetView from '../components/WidgetView.svelte'

  let ws = $state<WidgetSettings>(structuredClone($state.snapshot(app.settings!.widget)) as WidgetSettings)
  let data = $state<WidgetData | null>(null)
  let backdrop: 'light' | 'dark' | 'photo' = $state('photo')

  onMount(() => {
    api.widgetData().then((d) => (data = d))
  })

  let timer: ReturnType<typeof setTimeout> | undefined
  function commit() {
    clearTimeout(timer)
    timer = setTimeout(() => saveSettings({ widget: $state.snapshot(ws) as WidgetSettings }), 250)
  }
  function set<K extends keyof WidgetSettings>(k: K, v: WidgetSettings[K]) {
    ws[k] = v
    commit()
  }
  function move(i: number, d: -1 | 1) {
    const j = i + d
    if (j < 0 || j >= ws.items.length) return
    const items = [...ws.items]
    ;[items[i], items[j]] = [items[j], items[i]]
    ws.items = items
    commit()
  }
  function toggleItem(i: number, v: boolean) {
    ws.items[i].enabled = v
    commit()
  }
  function toggleProvider(p: Provider, v: boolean) {
    const all = (data?.providers ?? ['anthropic', 'openai']) as Provider[]
    const cur = ws.providers.length ? [...ws.providers] : [...all]
    const next = v ? [...new Set([...cur, p])] : cur.filter((x) => x !== p)
    ws.providers = next.length === all.length ? [] : next
    commit()
  }
  const providerOn = (p: Provider) => ws.providers.length === 0 || ws.providers.includes(p)
  async function reset() {
    const keep = { visible: ws.visible, x: ws.x, y: ws.y }
    const d: WidgetSettings = {
      visible: keep.visible, opacity: 0.85, size: 'm', scale: 1, x: keep.x, y: keep.y, anchor: 'bottom-right', auto_hide_fullscreen: true,
      layout: 'horizontal',
      items: (['primary', 'cost', 'limit_five_hour', 'limit_seven_day', 'tools', 'week_tokens', 'week_cost', 'month_cost', 'updated'] as WidgetItemKind[]).map((k, i) => ({ kind: k, enabled: i < 4 })),
      providers: [], primary_period: 'today', primary_metric: 'tokens', limit_style: 'ring', theme: 'system', accent: '',
      corner_radius: 14, border: true, shadow: false, show_labels: true, show_reset_time: false, warn_at: 70, high_at: 90,
      always_on_top: true, lock_position: false, click_action: 'open_dashboard',
    }
    ws = d
    commit()
  }
  const providers: Provider[] = ['anthropic', 'openai']
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
    <div class="stage {backdrop}" data-theme={ws.theme === 'system' ? undefined : ws.theme}>
      <WidgetView {data} {ws} />
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
        <span>{t('ws.providers')}</span>
        <div class="row">
          {#each providers as p (p)}
            <label class="chk"><Toggle checked={providerOn(p)} label={t(`provider.${p}`)} onchange={(v) => toggleProvider(p, v)} />{t(`provider.${p}`)}</label>
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
        <div class="row">
          {#each ['', '#2a78d6', '#1baf7a', '#eb6834', '#e87ba4', '#4a3aa7', '#52514e'] as c (c)}
            <button class="swatchbtn" class:sel={ws.accent === c} style="--c:{c || 'var(--accent)'}" aria-label={c || t('ws.accent.system')} title={c || t('ws.accent.system')} onclick={() => set('accent', c)}>
              {#if !c}<span>A</span>{/if}
            </button>
          {/each}
          <input type="color" class="picker" value={ws.accent || '#2a78d6'} aria-label={t('ws.accent.custom')} onchange={(e) => set('accent', e.currentTarget.value)} />
        </div>
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
      {#each ['border', 'shadow', 'show_labels', 'show_reset_time'] as const as key (key)}
        <div class="item">
          <span>{t(`ws.${key}`)}</span>
          <Toggle checked={ws[key]} label={t(`ws.${key}`)} onchange={(v) => set(key, v)} />
        </div>
      {/each}
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
      {#each ['visible', 'always_on_top', 'lock_position', 'auto_hide_fullscreen'] as const as key (key)}
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
  @media (max-width: 1240px) {
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
    border: 0.5px solid var(--hairline);
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
    min-width: 48px;
    text-align: right;
    color: var(--ink-2);
    font-size: 12.5px;
  }
  .swatchbtn {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 2px solid var(--surface);
    outline: 1px solid var(--hairline-strong);
    background: var(--c);
    display: grid;
    place-items: center;
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    padding: 0;
  }
  .swatchbtn.sel {
    outline: 2px solid var(--ink);
  }
  .picker {
    width: 30px;
    height: 26px;
    border: 0;
    background: transparent;
    padding: 0;
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
