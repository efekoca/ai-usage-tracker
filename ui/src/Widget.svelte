<script lang="ts">
  import { onMount } from 'svelte'
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { api, on, type Settings, type WidgetData } from './lib/api'
  import { fmtCompact, fmtDec, fmtMoney, fmtPct, t } from './lib/i18n.svelte'
  import { applyAppearance } from './lib/store.svelte'

  let data = $state<WidgetData | null>(null)
  let settings = $state<Settings | null>(null)

  async function load() {
    try {
      data = await api.widgetData()
    } catch {
      /* backend busy; next tick retries */
    }
  }
  onMount(() => {
    ;(async () => {
      const [info, s] = await Promise.all([api.appInfo(), api.getSettings()])
      settings = s
      applyAppearance(s, info)
      await load()
    })()
    const subs = [
      on('data-changed', load),
      on<Settings>('settings-changed', (s) => {
        settings = s
        applyAppearance(s, null)
        load()
      }),
    ]
    const id = setInterval(load, 60_000)
    return () => {
      clearInterval(id)
      subs.forEach((p) => p.then((u) => u()))
    }
  })

  // click opens the dashboard; a press that moves starts a window drag
  let down: { x: number; y: number } | null = null
  function pointerdown(e: PointerEvent) {
    if (e.button !== 0) return
    down = { x: e.screenX, y: e.screenY }
  }
  function pointermove(e: PointerEvent) {
    if (down && Math.hypot(e.screenX - down.x, e.screenY - down.y) > 4) {
      down = null
      getCurrentWindow().startDragging()
    }
  }
  function pointerup() {
    if (down) api.openMain()
    down = null
  }
  function menu(e: MouseEvent) {
    e.preventDefault()
    api.widgetMenu()
  }

  const zoom = $derived(settings?.widget.size === 's' ? 0.82 : settings?.widget.size === 'l' ? 1.24 : 1)
  const alpha = $derived(Math.round((settings?.widget.opacity ?? 0.85) * 100))
  const providerLimits = $derived(
    (data?.providers ?? []).map((p) => ({
      provider: p,
      five: data?.limits.find((l) => l.provider === p && l.window === 'five_hour') ?? null,
      week: data?.limits.find((l) => l.provider === p && l.window === 'seven_day') ?? null,
    })),
  )
  const R = 15
  const C = 2 * Math.PI * R
  const level = (pct: number) => (pct >= 90 ? 'var(--critical)' : pct >= 70 ? 'var(--serious)' : 'var(--accent)')
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="w"
  style="zoom:{zoom};--alpha:{alpha}%"
  onpointerdown={pointerdown}
  onpointermove={pointermove}
  onpointerup={pointerup}
  oncontextmenu={menu}
  title={t('widget.open')}
>
  {#if !data}
    <span class="muted small">…</span>
  {:else if data.providers.length === 0}
    <span class="muted small">{t('widget.noSources')}</span>
  {:else}
    <div class="today">
      <span class="label">{t('widget.today')}</span>
      <span class="big num">{fmtCompact(data.total_tokens)}</span>
      <span class="cost num">{fmtMoney(data.cost_usd)}{data.has_unpriced ? '*' : ''}</span>
    </div>
    <div class="rings">
      {#each providerLimits as pl (pl.provider)}
        {@const five = pl.five && pl.five.state === 'fresh' && pl.five.used_pct !== null ? pl.five.used_pct : null}
        {@const week = pl.week && pl.week.state === 'fresh' && pl.week.used_pct !== null ? pl.week.used_pct : null}
        <div class="ring" aria-label="{t(`provider.${pl.provider}`)} {five !== null ? fmtPct(five) : '—'}">
          <svg width="40" height="40" viewBox="0 0 40 40" aria-hidden="true">
            <circle cx="20" cy="20" r={R} class="track" />
            {#if five !== null}
              <circle cx="20" cy="20" r={R} class="arc" stroke={level(five)} stroke-dasharray="{(Math.min(100, five) / 100) * C} {C}" transform="rotate(-90 20 20)" />
            {/if}
            <text x="20" y="20" dy="0.35em" text-anchor="middle">{five !== null ? fmtDec(five) : '–'}</text>
          </svg>
          <div class="rl">
            <span class="pn">{t(`provider.${pl.provider}`)}</span>
            <span class="sub">{t('widget.5h')}{week !== null ? ` · ${t('widget.week')} ${fmtPct(week)}` : ''}{pl.five?.accuracy === 'estimated' ? ' ≈' : ''}</span>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  :global(html.widget),
  :global(html.widget body) {
    background: transparent !important;
  }
  .w {
    height: 100vh;
    box-sizing: border-box;
    margin: 0;
    padding: 12px 14px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--surface) var(--alpha), transparent);
    border: 0.5px solid var(--hairline-strong);
    display: flex;
    align-items: center;
    gap: 14px;
    user-select: none;
    overflow: hidden;
  }
  .today {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .label {
    font-size: 11px;
    color: var(--ink-2);
    font-weight: 500;
  }
  .big {
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.03em;
    line-height: 1.15;
  }
  .cost {
    font-size: 12px;
    color: var(--ink-2);
  }
  .rings {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-left: auto;
  }
  .ring {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ring svg {
    width: 32px;
    height: 32px;
  }
  .track {
    fill: none;
    stroke: var(--surface-press);
    stroke-width: 4;
  }
  .arc {
    fill: none;
    stroke-width: 4;
    stroke-linecap: round;
    transition: stroke-dasharray 600ms var(--ease);
  }
  text {
    font-size: 10.5px;
    font-weight: 650;
    fill: var(--ink);
  }
  .rl {
    display: flex;
    flex-direction: column;
    line-height: 1.2;
  }
  .pn {
    font-size: 12px;
    font-weight: 600;
  }
  .sub {
    font-size: 10.5px;
    color: var(--ink-2);
  }
</style>
