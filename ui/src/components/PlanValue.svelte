<script lang="ts">
  // Chat use is not in local logs, so the figure is a lower bound.
  import { untrack } from 'svelte'
  import { accountName, app, latest, providerColor, saveSettings, tint, markTint } from '../lib/store.svelte'
  import { api, providersOf, type PlansFile, type PlanValue, type Provider, type ProviderValue } from '../lib/api'
  import { cycleOf, priceKey, valueRow, valueSplit, type Cycle } from '../lib/plans'
  import { fmtDate, fmtDec, fmtInt, fmtMoney, i18n, localDate, parseNumber, t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import BrandIcon from './BrandIcon.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'
  import Select from './Select.svelte'

  let plans = $state<PlansFile | null>(null)
  let value = $state<PlanValue | null>(null)
  let failed = $state('')

  const today = $derived(new Date(`${app.today}T00:00:00`))
  // a billing period can start a day before the last 30 days
  const reach = $derived.by(() => {
    const month = localDate(new Date(today.getFullYear(), today.getMonth(), today.getDate() - 29))
    const first = Object.values(app.settings?.renewal_days ?? {}).map((d) => cycleOf(d, today).from).sort()[0]
    return first && first < month ? first : ''
  })

  $effect(() => {
    void app.tick
    const known = untrack(() => plans)
    const from = reach
    const to = app.today
    return latest(
      () => Promise.all([known ? Promise.resolve(known) : api.plans(), api.planValue(from ? { kind: 'custom', from, to } : undefined)]),
      ([p, v]) => {
        plans = p
        value = v
        failed = ''
      },
      (e) => (failed = String(e)),
    )
  })

  const providers = $derived(providersOf(app.settings?.enabled_sources ?? []))
  // a tool without use in the period would only fill a column with "no use"
  const used = $derived(providers.filter((p) => (value?.providers.find((x) => x.provider === p)?.events ?? 0) > 0))

  function setPlan(p: Provider, id: string) {
    saveSettings((c) => ({ plans: { ...c.plans, [p]: id } }))
  }
  function setPrice(p: Provider, raw: string) {
    // an empty field removes the custom price; text that is not a price changes nothing
    const v = parseNumber(raw)
    if (raw.trim() !== '' && !(v !== null && v > 0)) return
    saveSettings((c) => {
      const next = { ...c.plan_prices }
      if (v !== null && v > 0) next[p] = v
      else delete next[p]
      return { plan_prices: next }
    })
  }

  function setAccountPlan(id: string, plan: string) {
    saveSettings((c) => ({ account_plans: { ...c.account_plans, [id]: plan } }))
  }
  function setAccountPrice(key: string, raw: string) {
    const v = parseNumber(raw)
    if (!(v !== null && v > 0)) return
    saveSettings((c) => ({ account_prices: { ...c.account_prices, [key]: v } }))
  }

  const input = $derived({ plans, settings: app.settings, accounts: app.accounts, value, cycles: true, today })
  const row = (p: Provider) => valueRow(input, p)
  const split = (p: Provider) => valueSplit(input, p)
  const multiple = (r: number | null) => t('value.multiple', { n: fmtDec(r ?? 0, r !== null && r < 10 ? 1 : 0) })

  const W = 240
  const H = 64
  /** `span` days fill the width, so a billing period shows how far into it you are. */
  function path(cum: number[], top: number, span = cum.length) {
    if (!cum.length) return { line: '', area: '' }
    const x = (i: number) => (span <= 1 ? W : (i / (span - 1)) * W)
    const y = (v: number) => H - 2 - (v / top) * (H - 6)
    const pts = cum.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`)
    return { line: `M${pts.join('L')}`, area: `M0,${H}L${pts.join('L')}L${x(cum.length - 1).toFixed(1)},${H}Z` }
  }
  const shift = (day: string, n: number) => {
    const d = new Date(`${day}T12:00:00`)
    d.setDate(d.getDate() + n)
    return localDate(d)
  }
  const renewsOn = (c: Cycle) => t(c.renews.length === 1 ? 'value.renews' : 'value.renewsMany', { d: c.renews.join(', ') })
  const cycles = $derived(used.map((p) => (app.accounts.some((a) => a.provider === p) && split(p).parts.length ? split(p).cycle : row(p).cycle)))
  const lead = $derived(cycles.length && cycles.every(Boolean) ? 'value.leadCycle' : cycles.some(Boolean) ? 'value.leadMixed' : 'value.lead')
  const anyRenewal = $derived(Object.keys(app.settings?.renewal_days ?? {}).length > 0)
</script>

{#snippet usage(p: Provider, pv: ProviderValue | undefined)}
  {#if pv && pv.events > 0}
    {@const g = path(pv.cumulative, pv.cost_usd * 1.08 || 1)}
    <div class="figure">
      <div class="big num">{fmtMoney(pv.cost_usd)}</div>
      <span class="subtle small">{t('value.apiEq')}</span>
    </div>
    <svg class="spark" viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label="{t('value.chart.api')}: {fmtMoney(pv.cost_usd)}">
      <path d={g.area} class="area" style="--c:{tint(p) ?? providerColor[p]}" />
      <path d={g.line} class="line" style="--c:{tint(p) ?? providerColor[p]}" />
    </svg>
  {:else}
    <p class="muted small">{t('value.noUse')}</p>
  {/if}
{/snippet}

{#snippet compare(p: Provider, cost: number, share: number, ratio: number | null, day: number, cumulative: number[], note: string, cyc: Cycle | null)}
  {@const top = Math.max(cost, share) * 1.08 || 1}
  {@const g = path(cumulative, top, cyc?.days)}
  {@const planY = H - 2 - (share / top) * (H - 6)}
  <div class="figure">
    <div class="big num" title={t(cyc ? 'value.multipleHelpCycle' : 'value.multipleHelp')}>{multiple(ratio)}</div>
    <dl>
      <dt>{t(cyc ? 'value.apiEqCycle' : 'value.apiEq')}</dt>
      <dd class="num">{fmtMoney(cost)}</dd>
      <dt>{t(cyc ? 'value.planShareCycle' : 'value.planShare')}</dt>
      <dd class="num">{fmtMoney(share)}</dd>
      <dd class="note subtle small">{cyc ? `${note} · ${renewsOn(cyc)}` : note}</dd>
    </dl>
  </div>
  <svg class="spark" viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img"
    aria-label="{t('value.chart.api')}: {fmtMoney(cost)}; {t('value.chart.plan')}: {fmtMoney(share)}">
    <path d={g.area} class="area" style="--c:{tint(p) ?? providerColor[p]}" />
    <path d={g.line} class="line" style="--c:{tint(p) ?? providerColor[p]}" />
    <line x1="0" x2={W} y1={planY} y2={planY} class="plan" />
  </svg>
  <div class="legend small subtle">
    <span><i class="sw" style="background:{tint(p) ?? providerColor[p]}"></i>{t('value.chart.api')}</span>
    <span><i class="sw dash"></i>{t('value.chart.plan')}</span>
    <span class="spacer"></span>
    {#if cyc}
      <span title={t('value.cycleDay', { n: cyc.elapsed, of: cyc.days })}>{fmtDate(cyc.from, 'short')} – {fmtDate(shift(cyc.to, -1), 'short')}</span>
    {:else if value}<span>{fmtDate(value.dates[0], 'short')} – {fmtDate(value.dates[value.dates.length - 1], 'short')}</span>{/if}
  </div>
  <p class="verdict small icon-text">
    {#if (ratio ?? 0) >= 1}
      <Icon name="check" size={13} />
      {t('value.saved', { v: fmtMoney(cost - share) })} · {day >= 0 ? t(cyc ? 'value.breakevenCycle' : 'value.breakeven', { d: day + 1 }) : t('value.notYet')}
    {:else if cyc}
      <Icon name="info" size={13} />
      {t('value.toGo', { v: fmtMoney(share - cost), n: Math.max(0, cyc.days - cyc.elapsed) })}
    {:else}
      <Icon name="info" size={13} />
      {t('value.lost', { v: fmtMoney(share - cost) })}
    {/if}
  </p>
{/snippet}

<section class="card value">
  <div class="card-head">
    <h2>{t('value.title')}</h2>
    <AccuracyBadge kind="estimated" compact />
    <span class="spacer"></span>
    <span class="subtle small">{t(lead)}</span>
  </div>

  {#if failed && (!value || !plans)}
    <p class="muted small">{t('common.error', { e: failed })}</p>
  {:else if !value || !plans}
    <p class="muted small">{t('common.loading')}</p>
  {:else}
    {#if !used.length}<p class="muted small">{t('value.noUse')}</p>{/if}
    <div class="rows">
      {#each used as p (p)}
        {@const m = app.accounts.some((a) => a.provider === p) ? split(p) : null}
        {#if m && m.parts.length}
          {@const miss = m.missing}
          <div class="prow">
            <div class="who">
              <b class="brandname"><BrandIcon provider={p} color={markTint(p)} />{t(`provider.${p}`)}</b>
              {#each [...new Set(m.parts.map((x) => x.plan?.name).filter(Boolean))] as name (name)}<span class="pill">{name}</span>{/each}
            </div>
            {#if miss}
              {@render usage(p, m.pv)}
              {#key miss.id}
              <div class="pick">
                <span class="small"><b>{accountName(miss.id, p)}</b> · {miss.plan ? (miss.plan.price_note?.[i18n.lang] ?? t('value.enterPrice')) : t('value.pickAccount')}</span>
                {#if !miss.plan}
                  <Select
                    label="{accountName(miss.id, p)} · {t('limits.plan')}"
                    placeholder={t('value.pickPlan')}
                    minWidth={170}
                    options={plans.providers[p]?.plans.map((pl) => ({ value: pl.id, label: pl.name, sub: pl.relative })) ?? []}
                    onchange={(v) => setAccountPlan(miss.id, v)}
                  />
                {:else}
                  {@const plan = miss.plan}
                  {#each plan.price_options ?? [] as o (o)}
                    <button class="btn small" onclick={() => setAccountPrice(priceKey(miss.id, plan.id), String(o))}>{fmtMoney(o)}</button>
                  {/each}
                  <input class="field price" inputmode="decimal" placeholder={t('value.price')} aria-label="{accountName(miss.id, p)} · {t('value.price')}" onchange={(e) => setAccountPrice(priceKey(miss.id, plan.id), e.currentTarget.value)} />
                {/if}
              </div>
              {/key}
            {:else if m.share <= 0}
              <p class="muted small">{m.parts.every((x) => x.plan?.id === 'api') ? t('value.api') : t('value.free')}</p>
            {:else if m.events === 0}
              <p class="muted small">{t('value.noUse')}</p>
            {:else}
              {@render compare(p, m.cost, m.share, m.ratio, m.day, m.cumulative, `${t('value.perMonth', { price: fmtMoney(m.monthly) })} ${m.custom ? t('value.customPrice') : t('value.listPrice')} · ${t('value.accounts', { n: m.accounts })}`, m.cycle)}
              <ul class="parts small">
                {#each m.parts as x (x.id)}
                  <li>
                    <span class="pname">{accountName(x.id, p)}</span>
                    <span class="subtle">{x.plan?.name ?? ''}</span>
                    <span class="num">{fmtMoney(x.cost)}</span>
                    <span class="num subtle">{x.share ? multiple(x.cost / x.share) : '—'}</span>
                  </li>
                {/each}
                {#if m.unknown && m.unknown.events > 0}
                  <li>
                    <span class="pname muted">{accountName(null, p)}</span>
                    <span></span>
                    <span class="num">{fmtMoney(m.unknownCost)}</span>
                    <span class="num subtle">—</span>
                  </li>
                {/if}
              </ul>
              {#if m.unknown && m.unknown.events > 0}<p class="subtle small">{t(m.cycle ? 'value.unknownShareCycle' : 'value.unknownShare')}</p>{/if}
              {#if m.partly}<p class="subtle small">{t('value.cyclePartly')}</p>{/if}
              {#if m.unpriced > 0}<p class="subtle small">{t('value.unpriced', { n: fmtInt(m.unpriced) })}</p>{/if}
            {/if}
          </div>
        {:else}
        {@const r = row(p)}
        <div class="prow">
          <div class="who">
            <b class="brandname"><BrandIcon provider={p} color={markTint(p)} />{t(`provider.${p}`)}</b>
            {#if r.plan}<span class="pill">{r.plan.name}</span>{/if}
          </div>

          {#if !r.plan}
            {@render usage(p, r.pv)}
            <div class="pick">
              <span class="small">{t('value.pickLead')}</span>
              <Select
                label="{t(`provider.${p}`)} · {t('limits.plan')}"
                placeholder={t('value.pickPlan')}
                minWidth={170}
                options={plans.providers[p]?.plans.map((pl) => ({ value: pl.id, label: pl.name, sub: pl.relative })) ?? []}
                onchange={(v) => setPlan(p, v)}
              />
            </div>
          {:else if r.plan.id === 'api'}
            <p class="muted small">{t('value.api')}</p>
          {:else if r.price.usd === 0}
            <p class="muted small">{t('value.free')}</p>
          {:else if r.price.usd === null}
            {@render usage(p, r.pv)}
            <div class="pick">
              <span class="small">{r.plan.price_note?.[i18n.lang] ?? t('value.enterPrice')}</span>
              {#each r.plan.price_options ?? [] as o (o)}
                <button class="btn small" onclick={() => setPrice(p, String(o))}>{fmtMoney(o)}</button>
              {/each}
              <input class="field price" inputmode="decimal" placeholder={t('value.price')} aria-label={t('value.price')} onchange={(e) => setPrice(p, e.currentTarget.value)} />
            </div>
          {:else if !r.pv || r.pv.events === 0}
            <p class="muted small">{t('value.noUse')}</p>
          {:else}
            {@render compare(p, r.cost, r.share ?? 0, r.ratio, r.day, r.cumulative, `${t('value.perMonth', { price: fmtMoney(r.price.usd) })} ${r.price.custom ? t('value.customPrice') : t('value.listPrice')}`, r.cycle)}
            {#if r.pv.unpriced_events > 0}<p class="subtle small">{t('value.unpriced', { n: fmtInt(r.pv.unpriced_events) })}</p>{/if}
          {/if}
        </div>
        {/if}
      {/each}
    </div>
    <p class="subtle small foot">{t('value.notSavings')} {t('value.lowerBound')} {plans.prices_verified_at ? t('value.source', { d: fmtDate(plans.prices_verified_at) }) : ''}</p>
    {#if !anyRenewal && used.length}
      <p class="subtle small hint">
        <span>{t('value.cycleHint')}</span>
        <button class="btn ghost small" onclick={() => (app.view = 'limits')}>{t('value.cycleHintAction')}</button>
      </p>
    {/if}
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
    align-items: flex-start;
    margin: 0;
  }
  .verdict :global(svg) {
    color: var(--good-ink);
  }
  .price {
    width: 150px;
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    padding: 10px 12px;
    border: 0.5px dashed var(--hairline);
    border-radius: 10px;
    background: var(--surface-2);
  }
  .pick span {
    flex: 1 1 180px;
    color: var(--ink-2);
  }
  .parts {
    list-style: none;
    margin: 0;
    padding: 8px 0 0;
    border-top: 0.5px solid var(--hairline);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .parts li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 72px 52px;
    gap: 10px;
    align-items: baseline;
  }
  .parts .num {
    text-align: right;
  }
  .pname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .foot {
    margin: 14px 0 0;
  }
  .hint {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 10px;
    margin: 6px 0 0;
  }
  .spacer {
    flex: 1;
  }
</style>
