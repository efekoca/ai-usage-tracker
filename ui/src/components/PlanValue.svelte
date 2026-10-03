<script lang="ts">
  // Chat use is not in local logs, so the figure is a lower bound.
  import { untrack } from 'svelte'
  import { app, latest, saveSettings } from '../lib/store.svelte'
  import { api, type PlanDef, type PlansFile, type PlanValue, type Provider } from '../lib/api'
  import { fmtDate, fmtDec, fmtInt, fmtMoney, i18n, t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'

  let plans = $state<PlansFile | null>(null)
  let value = $state<PlanValue | null>(null)
  let failed = $state('')

  $effect(() => {
    void app.tick
    const known = untrack(() => plans)
    return latest(
      () => Promise.all([known ? Promise.resolve(known) : api.plans(), api.planValue()]),
      ([p, v]) => {
        plans = p
        value = v
        failed = ''
      },
      (e) => (failed = String(e)),
    )
  })

  const providers = $derived.by(() => {
    const s = new Set<Provider>()
    for (const src of app.settings?.enabled_sources ?? []) {
      if (src === 'codex') s.add('openai')
      else if (src !== 'chatgpt_desktop') s.add('anthropic')
    }
    return (['anthropic', 'openai'] as Provider[]).filter((p) => s.has(p))
  })

  const DAYS_PER_MONTH = 365.25 / 12
  function planOf(p: Provider): PlanDef | undefined {
    const id = app.settings?.plans[p]
    return id ? plans?.providers[p]?.plans.find((x) => x.id === id) : undefined
  }
  function priceOf(p: Provider): { usd: number | null; custom: boolean } {
    const own = app.settings?.plan_prices?.[p]
    if (typeof own === 'number' && own > 0) return { usd: own, custom: true }
    const list = planOf(p)?.monthly_usd
    return { usd: typeof list === 'number' ? list : null, custom: false }
  }
  function setPrice(p: Provider, raw: string) {
    const v = Number(raw.replace(',', '.'))
    saveSettings((c) => {
      const next = { ...c.plan_prices }
      if (v > 0) next[p] = v
      else delete next[p]
      return { plan_prices: next }
    })
  }

  function row(p: Provider) {
    const pv = value?.providers.find((x) => x.provider === p)
    const plan = planOf(p)
    const price = priceOf(p)
    const cost = pv?.cost_usd ?? 0
    const share = price.usd !== null ? (price.usd * (value?.dates.length ?? 30)) / DAYS_PER_MONTH : null
    const ratio = share && share > 0 ? cost / share : null
    const day = share !== null && pv ? pv.cumulative.findIndex((c) => c >= share) : -1
    return { pv, plan, price, cost, share, ratio, day }
  }

  const W = 240
  const H = 64
  function path(cum: number[], top: number) {
    if (!cum.length) return { line: '', area: '' }
    const x = (i: number) => (cum.length === 1 ? W : (i / (cum.length - 1)) * W)
    const y = (v: number) => H - 2 - (v / top) * (H - 6)
    const pts = cum.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`)
    return { line: `M${pts.join('L')}`, area: `M0,${H}L${pts.join('L')}L${W},${H}Z` }
  }
</script>

<section class="card value">
  <div class="card-head">
    <h2>{t('value.title')}</h2>
    <AccuracyBadge kind="estimated" compact />
    <span class="spacer"></span>
    <span class="subtle small">{t('value.lead')}</span>
  </div>

  {#if failed && (!value || !plans)}
    <p class="muted small">{t('common.error', { e: failed })}</p>
  {:else if !value || !plans}
    <p class="muted small">{t('common.loading')}</p>
  {:else}
    <div class="rows">
      {#each providers as p (p)}
        {@const r = row(p)}
        <div class="prow">
          <div class="who">
            <b>{t(`provider.${p}`)}</b>
            {#if r.plan}<span class="pill">{r.plan.name}</span>{/if}
          </div>

          {#if !r.plan}
            <p class="muted small">{t('value.choosePlan')} <button class="link" onclick={() => (app.view = 'limits')}>{t('value.choose')}</button></p>
          {:else if r.plan.id === 'api'}
            <p class="muted small">{t('value.api')}</p>
          {:else if r.price.usd === 0}
            <p class="muted small">{t('value.free')}</p>
          {:else if r.price.usd === null}
            <div class="enter">
              <p class="muted small">{r.plan.price_note?.[i18n.lang] ?? t('value.enterPrice')}</p>
              <div class="row wrap">
                {#each r.plan.price_options ?? [] as o (o)}
                  <button class="btn small" onclick={() => setPrice(p, String(o))}>{fmtMoney(o)}</button>
                {/each}
                <input class="field price" inputmode="decimal" placeholder={t('value.price')} aria-label={t('value.price')} onchange={(e) => setPrice(p, e.currentTarget.value)} />
              </div>
            </div>
          {:else if !r.pv || r.pv.events === 0}
            <p class="muted small">{t('value.noUse')}</p>
          {:else}
            {@const top = Math.max(r.cost, r.share ?? 0) * 1.08 || 1}
            {@const g = path(r.pv.cumulative, top)}
            {@const planY = H - 2 - ((r.share ?? 0) / top) * (H - 6)}
            <div class="figure">
              <div class="big num" title={t('value.multipleHelp')}>{t('value.multiple', { n: fmtDec(r.ratio ?? 0, r.ratio !== null && r.ratio < 10 ? 1 : 0) })}</div>
              <dl>
                <dt>{t('value.apiEq')}</dt>
                <dd class="num">{fmtMoney(r.cost)}</dd>
                <dt>{t('value.planShare')}</dt>
                <dd class="num">{fmtMoney(r.share ?? 0)}</dd>
                <dd class="note subtle small">{fmtMoney(r.price.usd)}/{i18n.lang === 'tr' ? 'ay' : 'mo'} {r.price.custom ? t('value.customPrice') : t('value.listPrice')}</dd>
              </dl>
            </div>
            <svg class="spark" viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img"
              aria-label="{t('value.chart.api')}: {fmtMoney(r.cost)}; {t('value.chart.plan')}: {fmtMoney(r.share ?? 0)}">
              <path d={g.area} class="area" style="--c:var(--{p === 'openai' ? 's1' : 's2'})" />
              <path d={g.line} class="line" style="--c:var(--{p === 'openai' ? 's1' : 's2'})" />
              <line x1="0" x2={W} y1={planY} y2={planY} class="plan" />
            </svg>
            <div class="legend small subtle">
              <span><i class="sw" style="background:var(--{p === 'openai' ? 's1' : 's2'})"></i>{t('value.chart.api')}</span>
              <span><i class="sw dash"></i>{t('value.chart.plan')}</span>
              <span class="spacer"></span>
              <span>{fmtDate(value.dates[0], 'short')} – {fmtDate(value.dates[value.dates.length - 1], 'short')}</span>
            </div>
            <p class="verdict small">
              {#if (r.ratio ?? 0) >= 1}
                <Icon name="check" size={13} />
                {t('value.saved', { v: fmtMoney(r.cost - (r.share ?? 0)) })} · {r.day >= 0 ? t('value.breakeven', { d: r.day + 1 }) : t('value.notYet')}
              {:else}
                <Icon name="info" size={13} />
                {t('value.lost', { v: fmtMoney((r.share ?? 0) - r.cost) })}
              {/if}
            </p>
            {#if r.pv.unpriced_events > 0}<p class="subtle small">{t('value.unpriced', { n: fmtInt(r.pv.unpriced_events) })}</p>{/if}
          {/if}
        </div>
      {/each}
    </div>
    <p class="subtle small foot">{t('value.notSavings')} {t('value.lowerBound')} {plans.prices_verified_at ? t('value.source', { d: fmtDate(plans.prices_verified_at) }) : ''}</p>
  {/if}
</section>

<style>
  section {
    margin-bottom: 16px;
  }
  .card-head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .rows {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 16px 28px;
  }
  .prow {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pill {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-press);
    color: var(--ink-2);
  }
  .figure {
    display: flex;
    align-items: center;
    gap: 20px;
  }
  .big {
    font-size: 34px;
    font-weight: 700;
    letter-spacing: -0.03em;
    line-height: 1;
    min-width: 84px;
  }
  dl {
    flex: 1;
    min-width: 0;
    margin: 0;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 12px;
    font-size: 13px;
  }
  dt {
    color: var(--ink-2);
  }
  dd {
    margin: 0;
    text-align: right;
    white-space: nowrap;
  }
  dd.note {
    grid-column: 1 / -1;
    white-space: normal;
    margin-top: -1px;
  }
  .spark {
    width: 100%;
    height: 64px;
    display: block;
  }
  .area {
    fill: color-mix(in srgb, var(--c) 14%, transparent);
  }
  .line {
    fill: none;
    stroke: var(--c);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
  .plan {
    stroke: var(--ink-2);
    stroke-width: 1.5;
    stroke-dasharray: 5 4;
    vector-effect: non-scaling-stroke;
  }
  .legend {
    display: flex;
    gap: 14px;
    align-items: center;
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .sw {
    width: 10px;
    height: 3px;
    border-radius: 2px;
  }
  .sw.dash {
    background: repeating-linear-gradient(90deg, var(--ink-2) 0 4px, transparent 4px 7px);
  }
  .verdict {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 0;
  }
  .verdict :global(svg) {
    color: var(--good-ink);
  }
  .enter .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .price {
    width: 150px;
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .foot {
    margin: 14px 0 0;
  }
  .spacer {
    flex: 1;
  }
</style>
