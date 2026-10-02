<script lang="ts">
  // The widget's content, driven entirely by WidgetSettings. Used by the widget window and by
  // the live preview on the Widget settings page.
  import type { Provider, WidgetData, WidgetItemKind, WidgetSettings } from '../lib/api'
  import { fmtCompact, fmtDec, fmtDuration, fmtMoney, fmtPct, fmtTime, t, toolLabel } from '../lib/i18n.svelte'
  import { toolColor } from '../lib/store.svelte'

  let { data, ws, root = $bindable() }: { data: WidgetData | null; ws: WidgetSettings; root?: HTMLElement } = $props()

  const on = (k: WidgetItemKind) => ws.items.some((i) => i.kind === k && i.enabled)
  const order = $derived(ws.items.filter((i) => i.enabled).map((i) => i.kind))
  const limitKinds: WidgetItemKind[] = ['limit_five_hour', 'limit_seven_day']
  const statKinds = $derived(order.filter((k) => !limitKinds.includes(k)))
  const limitOrder = $derived(order.filter((k) => limitKinds.includes(k)))

  const period = $derived(data ? data[ws.primary_period] : null)
  const providers = $derived(
    (data?.providers ?? []).filter((p) => ws.providers.length === 0 || ws.providers.includes(p)) as Provider[],
  )
  const periodLabel = $derived(t(`widget.period.${ws.primary_period}`))

  function limit(p: Provider, kind: WidgetItemKind) {
    const w = kind === 'limit_five_hour' ? 'five_hour' : 'seven_day'
    const l = data?.limits.find((x) => x.provider === p && x.window === w)
    if (!l) return null
    const known = (l.state === 'fresh' || l.state === 'behind') && l.used_pct !== null
    // 'behind' readings are lower bounds: usage happened after them
    return { pct: known ? Math.max(0, Math.min(100, l.used_pct as number)) : null, min: l.state === 'behind' ? '≥' : '', resets: l.resets_at, estimated: l.accuracy === 'estimated', captured: l.accuracy === 'captured' }
  }
  const level = (pct: number) => (pct >= ws.high_at ? 'var(--critical)' : pct >= ws.warn_at ? 'var(--serious)' : 'var(--w-accent)')
  const winShort = (k: WidgetItemKind) => (k === 'limit_five_hour' ? t('widget.5h') : t('widget.week'))

  let now = $state(Date.now())
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 30_000)
    return () => clearInterval(id)
  })

  const R = 15
  const C = 2 * Math.PI * R
  const primaryValue = $derived(period ? (ws.primary_metric === 'tokens' ? fmtCompact(period.tokens) : fmtMoney(period.cost_usd)) : '–')
  const secondaryValue = $derived(period ? (ws.primary_metric === 'tokens' ? fmtMoney(period.cost_usd) : fmtCompact(period.tokens) + ' ' + t('metric.tokens').toLowerCase()) : '')
</script>

{#snippet stat(kind: WidgetItemKind)}
  {#if kind === 'primary'}
    <div class="primary">
      {#if ws.show_labels}<span class="label">{periodLabel}</span>{/if}
      <span class="big num">{primaryValue}</span>
    </div>
  {:else if kind === 'cost'}
    <span class="secondary num">{secondaryValue}</span>
  {:else if kind === 'tools' && period}
    <div class="tools">
      {#each period.tools as tl (tl.tool)}
        <span class="tool"><i style="background:{toolColor[tl.tool]}"></i>{#if ws.show_labels}<span class="tn">{toolLabel(tl.tool)}</span>{/if}<b class="num">{ws.primary_metric === 'tokens' ? fmtCompact(tl.tokens) : fmtMoney(tl.cost_usd)}</b></span>
      {/each}
    </div>
  {:else if kind === 'week_tokens' && data}
    <span class="mini">{#if ws.show_labels}<span class="ml">{t('widget.period.days7')}</span>{/if}<b class="num">{fmtCompact(data.days7.tokens)}</b></span>
  {:else if kind === 'week_cost' && data}
    <span class="mini">{#if ws.show_labels}<span class="ml">{t('widget.period.days7')}</span>{/if}<b class="num">{fmtMoney(data.days7.cost_usd)}</b></span>
  {:else if kind === 'month_cost' && data}
    <span class="mini">{#if ws.show_labels}<span class="ml">{t('widget.period.month1')}</span>{/if}<b class="num">{fmtMoney(data.month1.cost_usd)}</b></span>
  {:else if kind === 'updated' && data}
    <span class="updated">{t('widget.updated', { t: fmtTime(data.updated_ms) })}</span>
  {/if}
{/snippet}

{#snippet providerLimits(p: Provider, kinds: WidgetItemKind[])}
  <!-- one row per provider: the first enabled window leads, the others follow inline -->
  {@const rows = kinds.map((k) => ({ k, l: limit(p, k) })).filter((x) => x.l !== null) as { k: WidgetItemKind; l: NonNullable<ReturnType<typeof limit>> }[]}
  {#if rows.length}
    {@const lead = rows[0]}
    {#if ws.limit_style === 'ring'}
      <div class="ring" aria-label="{t(`provider.${p}`)} {rows.map((r) => `${winShort(r.k)} ${r.l.pct !== null ? r.l.min + fmtPct(r.l.pct) : '—'}`).join(', ')}">
        <svg viewBox="0 0 40 40" aria-hidden="true">
          <circle cx="20" cy="20" r={R} class="track" />
          {#if lead.l.pct !== null}
            <circle cx="20" cy="20" r={R} class="arc" stroke={level(lead.l.pct)} stroke-dasharray="{(lead.l.pct / 100) * C} {C}" transform="rotate(-90 20 20)" />
          {/if}
          <text x="20" y="20" dy="0.35em" text-anchor="middle">{lead.l.pct !== null ? lead.l.min + fmtDec(lead.l.pct) : '–'}</text>
        </svg>
        <div class="rl">
          {#if ws.show_labels}<span class="pn">{t(`provider.${p}`)}</span>{/if}
          <span class="sub">
            {winShort(lead.k)}{lead.l.estimated ? ' ≈' : ''}{ws.show_reset_time && lead.l.resets ? ` · ${fmtDuration(lead.l.resets * 1000 - now)}` : ''}
            {#each rows.slice(1) as r (r.k)}
              · {winShort(r.k)} <b class="num" style="color:{r.l.pct !== null && r.l.pct >= ws.warn_at ? level(r.l.pct) : 'inherit'}">{r.l.pct !== null ? r.l.min + fmtPct(r.l.pct) : '—'}</b>
            {/each}
          </span>
        </div>
      </div>
    {:else if ws.limit_style === 'bar'}
      <div class="barrow">
        {#if ws.show_labels}<span class="pn">{t(`provider.${p}`)}</span>{/if}
        {#each rows as r (r.k)}
          <div class="bh">
            <span class="sub">{winShort(r.k)}{ws.show_reset_time && r.l.resets ? ` · ${fmtDuration(r.l.resets * 1000 - now)}` : ''}</span>
            <span class="pv num">{r.l.pct !== null ? r.l.min + fmtPct(r.l.pct) : '—'}{r.l.estimated ? ' ≈' : ''}</span>
          </div>
          <div class="bt"><span style="width:{r.l.pct ?? 0}%;background:{r.l.pct !== null ? level(r.l.pct) : 'transparent'}"></span></div>
        {/each}
      </div>
    {:else}
      <span class="textlimit">
        {#if ws.show_labels}<span class="pn">{t(`provider.${p}`)}</span>{/if}
        {#each rows as r, i (r.k)}
          {#if i > 0}<span class="sub">·</span>{/if}
          <span class="sub">{winShort(r.k)}</span>
          <b class="num" style="color:{r.l.pct !== null && r.l.pct >= ws.warn_at ? level(r.l.pct) : 'inherit'}">{r.l.pct !== null ? r.l.min + fmtPct(r.l.pct) : '—'}</b>
        {/each}
      </span>
    {/if}
  {/if}
{/snippet}

<div
  bind:this={root}
  class="w {ws.layout}"
  class:bordered={ws.border}
  class:shadowed={ws.shadow}
  class:proportional={!ws.tabular_nums}
  style="zoom:{ws.scale};--alpha:{Math.round(ws.opacity * 100)}%;--radius:{ws.corner_radius}px;--ts:{ws.text_scale};--ns:{ws.number_scale};--nw:{ws.number_weight};{ws.font_family ? `--wfont:'${ws.font_family}', var(--font);` : ''}{ws.accent ? `--w-accent:${ws.accent};` : ''}"
>
  {#if !data}
    <span class="sub">…</span>
  {:else if providers.length === 0}
    <span class="sub">{t('widget.noSources')}</span>
  {:else if ws.layout === 'line'}
    {#each order as kind (kind)}
      {#if kind === limitOrder[0]}
        {#each providers as p (p)}{@render providerLimits(p, limitOrder)}{/each}
      {:else if !limitKinds.includes(kind)}
        {@render stat(kind)}
      {/if}
    {/each}
  {:else}
    {#if statKinds.length}
      <div class="col stats">
        {#each statKinds as kind (kind)}{@render stat(kind)}{/each}
      </div>
    {/if}
    {#if limitOrder.length && (on('limit_five_hour') || on('limit_seven_day'))}
      <div class="col limits" class:barstyle={ws.limit_style === 'bar'}>
        {#each providers as p (p)}{@render providerLimits(p, limitOrder)}{/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .w {
    --w-accent: var(--accent);
    display: inline-flex;
    gap: 16px;
    padding: 12px 14px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--surface) var(--alpha), transparent);
    color: var(--ink);
    font-family: var(--wfont, var(--font));
    user-select: none;
    box-sizing: border-box;
    white-space: nowrap;
  }
  .w.proportional .num,
  .w.proportional text {
    font-variant-numeric: proportional-nums;
  }
  .w.bordered {
    border: 0.5px solid var(--hairline-strong);
  }
  .w.shadowed {
    /* room for the shadow inside the self-sized window */
    margin: 10px 14px 18px;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12), 0 8px 24px rgba(0, 0, 0, 0.18);
  }
  .w.horizontal {
    flex-direction: row;
    align-items: center;
  }
  .w.vertical {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }
  .w.line {
    flex-direction: row;
    align-items: center;
    gap: 12px;
    padding: 7px 12px;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .limits {
    gap: 6px;
  }
  .limits.barstyle {
    gap: 8px;
    min-width: 150px;
  }
  .primary {
    display: flex;
    flex-direction: column;
  }
  .label,
  .ml {
    font-size: calc(11px * var(--ts));
    font-weight: 500;
    color: var(--ink-2);
  }
  .big {
    font-size: calc(26px * var(--ns));
    font-weight: var(--nw);
    letter-spacing: -0.03em;
    line-height: 1.1;
  }
  .line .big {
    font-size: calc(16px * var(--ns));
  }
  .line .primary {
    flex-direction: row;
    align-items: baseline;
    gap: 6px;
  }
  .secondary {
    font-size: calc(12.5px * var(--ts));
    color: var(--ink-2);
  }
  .mini {
    display: inline-flex;
    gap: 6px;
    align-items: baseline;
    font-size: calc(12px * var(--ts));
  }
  .tools {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .line .tools {
    flex-direction: row;
    gap: 10px;
  }
  .tool {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: calc(12px * var(--ts));
  }
  .tool i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }
  .tn {
    color: var(--ink-2);
  }
  .updated {
    font-size: calc(10.5px * var(--ts));
    color: var(--ink-3);
  }
  .ring {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .ring svg {
    width: calc(34px * var(--ts));
    height: calc(34px * var(--ts));
    flex: none;
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
    font-weight: var(--nw);
    fill: var(--ink);
  }
  .rl {
    display: flex;
    flex-direction: column;
    line-height: 1.2;
  }
  .pn {
    font-size: calc(12px * var(--ts));
    font-weight: 600;
  }
  .sub {
    font-size: calc(10.5px * var(--ts));
    color: var(--ink-2);
    font-weight: 400;
  }
  .barrow {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .bh {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    align-items: baseline;
  }
  .pv {
    font-size: calc(12px * var(--ts));
    font-weight: var(--nw);
  }
  .bt {
    height: 5px;
    border-radius: 3px;
    background: var(--surface-press);
    overflow: hidden;
  }
  .bt span {
    display: block;
    height: 100%;
    border-radius: 3px;
    transition: width 600ms var(--ease);
  }
  .textlimit {
    display: inline-flex;
    gap: 4px;
    align-items: baseline;
    font-size: calc(12px * var(--ts));
  }
</style>
