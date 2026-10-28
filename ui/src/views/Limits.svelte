<script lang="ts">
  import { onMount } from 'svelte'
  import { accountName, accountOf, app, saveSettings, markTint } from '../lib/store.svelte'
  import { api, providersOf, type PlansFile, type Provider, type Threshold } from '../lib/api'
  import { fmtCompact, fmtDate, fmtInput, fmtMoney, fmtPct, i18n, limitName, parseNumber, t, windowLabel } from '../lib/i18n.svelte'
  import LimitMeter from '../components/LimitMeter.svelte'
  import AccuracyBadge from '../components/AccuracyBadge.svelte'
  import Icon from '../components/Icon.svelte'
  import BrandIcon from '../components/BrandIcon.svelte'
  import Segmented from '../components/Segmented.svelte'
  import Select from '../components/Select.svelte'
  import LimitHistory from '../components/LimitHistory.svelte'
  import LimitNotices from '../components/LimitNotices.svelte'

  let plans: PlansFile | null = $state(null)
  onMount(async () => {
    plans = await api.plans()
  })

  const providers = $derived(providersOf(app.settings?.enabled_sources ?? []))

  function setPlan(p: Provider, id: string) {
    saveSettings((c) => {
      const next = { ...c.plans }
      if (id) next[p] = id
      else delete next[p]
      return { plans: next }
    })
  }

  let draft: Threshold = $state({ provider: 'anthropic', window: 'five_hour', tokens: null, cost_usd: null })
  let draftKind: 'cost' | 'tokens' = $state('cost')
  let draftValue = $state('')
  const draftProvider = $derived<Provider | undefined>(providers.includes(draft.provider) ? draft.provider : providers[0])

  const reloadLimits = () => api.limits().then((l) => (app.limits = l))
  function addThreshold() {
    const v = parseNumber(draftValue) ?? 0
    if (!(v > 0) || !draftProvider) return
    const th: Threshold = { provider: draftProvider, window: draft.window, tokens: draftKind === 'tokens' ? Math.round(v) : null, cost_usd: draftKind === 'cost' ? v : null }
    saveSettings((c) => ({ thresholds: [...c.thresholds.filter((x) => !(x.provider === th.provider && x.window === th.window)), th] })).then(reloadLimits)
    draftValue = ''
  }
  function removeThreshold(th: Threshold) {
    saveSettings((c) => ({ thresholds: c.thresholds.filter((x) => !(x.provider === th.provider && x.window === th.window)) })).then(reloadLimits)
  }
  function rename(id: string, name: string) {
    saveSettings((c) => {
      const next = { ...c.account_names }
      if (name.trim()) next[id] = name.trim()
      else delete next[id]
      return { account_names: next }
    })
  }
  const planNote = (p: Provider) => plans?.providers[p]?.plans.find((x) => x.id === app.settings?.plans[p])
  function setPrice(p: Provider, raw: string): boolean {
    // an empty field removes the custom price; text that is not a price changes nothing
    const v = parseNumber(raw)
    if (raw.trim() !== '' && !(v !== null && v > 0)) return false
    saveSettings((c) => {
      const next = { ...c.plan_prices }
      if (v !== null && v > 0) next[p] = v
      else delete next[p]
      return { plan_prices: next }
    })
    return true
  }
</script>

<header class="bar">
  <h1>{t('limits.title')}</h1>
  <Segmented label={t('limits.title')} bind:value={app.limitsTab} options={[{ value: 'current', label: t('limits.tab.current') }, { value: 'history', label: t('limits.tab.history') }]} />
  <span class="spacer"></span>
  {#if app.limitsTab === 'current'}
    <span class="subtle small" title={t('limits.mode.help')}>{t('limits.mode')}</span>
    <Segmented label={t('limits.mode')} value={app.settings?.limit_display ?? 'used'} options={[{ value: 'used', label: t('limits.mode.used') }, { value: 'remaining', label: t('limits.mode.remaining') }]} onchange={(v) => saveSettings({ limit_display: v })} />
  {/if}
</header>

{#if app.limitsTab === 'history'}
  <LimitHistory />
{:else}
{#if plans}
  <section class="card plans">
    {#each providers as p (p)}
      <div class="plan-row">
        <label for="plan-{p}"><b class="brandname"><BrandIcon provider={p} color={markTint(p)} />{plans.providers[p].label}</b> · {t('limits.plan')}</label>
        <Select
          id="plan-{p}"
          label="{plans.providers[p].label} · {t('limits.plan')}"
          value={app.settings?.plans[p] ?? ''}
          minWidth={200}
          options={[{ value: '', label: t('onb.plan.none') }, ...plans.providers[p].plans.map((pl) => ({ value: pl.id, label: pl.name, sub: pl.relative }))]}
          onchange={(v) => setPlan(p, v)}
        />
        {#if planNote(p)}
          {@const pl = planNote(p)!}
          <div class="extra">
          {#if pl.id !== 'api' && pl.monthly_usd !== 0}
            {@const own = app.settings?.plan_prices?.[p]}
            <label class="price">
              <span class="subtle small">{t('value.price')}</span>
              <input class="field" inputmode="decimal" value={fmtInput(own)} placeholder={typeof pl.monthly_usd === 'number' ? `${fmtInput(pl.monthly_usd)} (${t('value.listPrice')})` : '—'} onchange={(e) => { if (!setPrice(p, e.currentTarget.value)) e.currentTarget.value = fmtInput(own) }} title={pl.price_note?.[i18n.lang] ?? ''} />
            </label>
          {/if}
          <span class="subtle small">{pl.note ? pl.note[i18n.lang] : ''}</span>
          <button class="btn ghost small" onclick={() => api.openUrl(pl.source)}><Icon name="external" size={13} />{t('settings.pricing.sources')}</button>
          </div>
        {/if}
      </div>
    {/each}
    <p class="subtle small">{plans.notes[i18n.lang]} {providers.map((p) => plans?.providers[p]?.shared_pool_note?.[i18n.lang] ?? '').filter(Boolean).join(' ')}</p>
  </section>
{/if}

{#each (['anthropic', 'openai'] as const).filter((p) => app.accounts.some((a) => a.provider === p)) as p (p)}
  <section class="card accounts">
    <h2><BrandIcon provider={p} color={markTint(p)} />{t(`accounts.title.${p}`)}</h2>
    <p class="subtle small help">{t(`accounts.help.${p}`)}</p>
    {#each app.accounts.filter((a) => a.provider === p) as a (a.id ?? '')}
      <div class="acct-row">
        {#if a.id !== null}
          {@const id = a.id}
          <input
            class="field"
            maxlength="32"
            value={app.settings?.account_names?.[id] ?? ''}
            placeholder={accountName(id, p)}
            aria-label="{t('accounts.name')}: {accountName(id, p)}"
            onchange={(e) => rename(id, e.currentTarget.value)}
          />
        {:else}
          <span class="unknown" title={t('accounts.unknown.help')}>{accountName(null, p)}</span>
        {/if}
        {#if a.active}<span class="pill in-use">{t('accounts.inUse')}</span>{/if}
        <span class="spacer"></span>
        <span class="subtle small">{t('accounts.seen', { from: fmtDate(a.first_ms, 'short'), to: fmtDate(a.last_ms, 'short') })}</span>
        <span class="num small">{fmtCompact(a.requests)} {t('accounts.requests')}</span>
      </div>
    {/each}
  </section>
{/each}

<LimitNotices />

{#if app.limits.length === 0}
  <div class="banner"><Icon name="info" size={16} />{t('limits.none')}</div>
{/if}

<div class="cards">
  {#each app.limits as l (l.provider + (l.account ?? '') + l.limit_id + l.window + l.source)}
    {@const acct = accountOf(l)}
    <section class="card limit">
      <div class="who">
        <!-- brand names: uppercase with English rules so Antigravity does not become ANTİGRAVİTY in Turkish -->
        <span class="prov" lang="en"><BrandIcon provider={l.provider} size={13} color={markTint(l.provider)} />{limitName(l.provider, l.limit_id)}</span>
        {#if acct}<span class="acct" title={acct}>{acct}</span>{/if}
        {#if l.plan}<span class="pill">{l.plan}</span>{/if}
        {#if acct && l.active}<span class="pill in-use">{t('accounts.inUse')}</span>{/if}
      </div>
      <LimitMeter window={l.window} used={l.used_pct} state={l.state} accuracy={l.accuracy} resetsAt={l.resets_at} observedMs={l.observed_ms} source={l.source} sinceTokens={l.usage_since.total_tokens} provider={l.provider} forecast={l.forecast} />
      <div class="usage">
        <div class="small muted">{t('limits.windowUsage')}</div>
        <div class="row">
          <b class="num">{fmtCompact(l.window_usage.total_tokens)}</b><span class="muted small">{t('metric.tokens')}</span>
          <span class="spacer"></span>
          <b class="num">{fmtMoney(l.window_usage.cost_usd)}</b><span class="muted small">{t('metric.cost')}</span>
        </div>
      </div>
      {#if l.projects.length || l.unlogged_pct !== null}
        <div class="projects">
          <div class="row small muted" title={t('limits.byProject.help')}>
            {t('limits.byProject')}
            <AccuracyBadge kind="estimated" compact />
          </div>
          <ul>
            {#each l.projects.slice(0, 6) as p (p.project_id ?? -1)}
              <li>
                <span class="pname">{p.project_id === null ? t('projects.noProject') : p.hidden ? `${t('projects.hidden')} #${p.project_id}` : p.name}</span>
                <span class="bar" aria-hidden="true"><span style="width:{p.share * 100}%"></span></span>
                <span class="num small">{p.estimated_pct !== null ? `≈ ${fmtPct(p.estimated_pct, 1)}` : `${fmtPct(p.share * 100)}`}</span>
              </li>
            {/each}
            {#if l.unlogged_pct !== null && l.used_pct}
              <li title={t('limits.unlogged.help')}>
                <span class="pname muted">{t('limits.unlogged')}</span>
                <span class="bar" aria-hidden="true"><span style="width:{(l.unlogged_pct / l.used_pct) * 100}%"></span></span>
                <span class="num small">≈ {fmtPct(l.unlogged_pct, 1)}</span>
              </li>
            {/if}
          </ul>
        </div>
      {/if}
    </section>
  {/each}
</div>

{#if app.settings?.enabled_sources.includes('opencode') && app.limits.some((l) => l.provider === 'anthropic' || l.provider === 'openai')}
  <p class="subtle small oc-note"><Icon name="info" size={13} />{t('limits.opencode')}</p>
{/if}

<section class="card">
  <h2>{t('limits.thresholds')}</h2>
  <p class="subtle small help">{t('limits.thresholds.help')}</p>
  {#each app.settings?.thresholds ?? [] as th (th.provider + th.window)}
    {@const name = `${t(`provider.${th.provider}`)} · ${windowLabel(th.window)}`}
    <div class="list-row">
      <span>{name}</span>
      <span class="spacer"></span>
      <span class="num">{th.cost_usd ? fmtMoney(th.cost_usd) : `${fmtCompact(th.tokens ?? 0)} ${t('metric.tokens')}`}</span>
      <button class="btn ghost" aria-label="{t('common.remove')}: {name}" onclick={() => removeThreshold(th)}><Icon name="trash" size={15} /></button>
    </div>
  {/each}
  <div class="add">
    <Select label={t('history.filter.provider')} value={draftProvider ?? ''} disabled={!draftProvider} options={providers.map((p) => ({ value: p, label: t(`provider.${p}`) }))} onchange={(v) => (draft.provider = v as Provider)} />
    <Select label={t('history.filter.window')} value={draft.window} options={[{ value: 'five_hour', label: windowLabel('five_hour') }, { value: 'seven_day', label: windowLabel('seven_day') }]} onchange={(v) => (draft.window = v)} />
    <Select label={t('limits.thresholdKind')} value={draftKind} options={[{ value: 'cost', label: `${t('metric.cost')} (USD)` }, { value: 'tokens', label: t('metric.tokens') }]} onchange={(v) => (draftKind = v as 'cost' | 'tokens')} />
    <input class="field" inputmode="decimal" bind:value={draftValue} placeholder={draftKind === 'cost' ? '50' : '5000000'} aria-label={draftKind === 'cost' ? t('limits.thresholdCost') : t('limits.thresholdTokens')} />
    <button class="btn" onclick={addThreshold} disabled={!draftProvider}><Icon name="plus" size={14} />{t('limits.addThreshold')}</button>
  </div>
</section>

{/if}

<style>
  .price {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .price .field {
    width: 128px;
  }
  header.bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
    padding: 16px 0 14px;
  }
  header .spacer {
    flex: 1;
  }
  section {
    margin-bottom: 16px;
  }
  .plans {
    display: flex;
    flex-direction: column;
    gap: 10px;
    container: plans / inline-size;
  }
  /* name, plan list, then price and links; on a narrow window the extras move under the list */
  .plan-row {
    display: grid;
    grid-template-columns: 196px 240px minmax(0, 1fr);
    align-items: center;
    gap: 8px 12px;
  }
  .extra {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
    min-width: 0;
  }
  @container plans (max-width: 820px) {
    .plan-row {
      grid-template-columns: 196px minmax(0, 240px) minmax(0, 1fr);
    }
    .extra {
      grid-column: 2 / -1;
    }
  }
  @container plans (max-width: 520px) {
    .plan-row {
      grid-template-columns: minmax(0, 1fr);
    }
    .extra {
      grid-column: 1;
    }
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 16px;
    margin-bottom: 16px;
  }
  .cards section {
    margin: 0;
  }
  .oc-note {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: -4px 0 16px;
    line-height: 1.5;
  }
  .oc-note :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .limit {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .who {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .prov {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    font-weight: 600;
    color: var(--ink-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .pill {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 6px;
    background: var(--surface-hover);
    color: var(--ink-2);
    text-transform: capitalize;
  }
  .acct {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 600;
  }
  .pill.in-use {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--ink);
    text-transform: none;
  }
  .accounts h2 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .acct-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
    padding: 6px 0;
    border-top: 0.5px solid var(--hairline);
  }
  .acct-row .field {
    width: 220px;
  }
  .unknown {
    font-weight: 600;
    color: var(--ink-2);
  }
  .usage {
    border-top: 0.5px solid var(--hairline);
    padding-top: 10px;
  }
  .usage .row {
    gap: 6px;
    align-items: baseline;
  }
  .projects ul {
    list-style: none;
    padding: 0;
    margin: 8px 0 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .projects li {
    display: grid;
    grid-template-columns: 1fr 80px 64px;
    gap: 10px;
    align-items: center;
    font-size: 13px;
  }
  .pname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .projects .bar {
    padding: 0;
    height: 5px;
    border-radius: 3px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .projects .bar span {
    display: block;
    height: 100%;
    background: var(--ink-3);
  }
  .projects .num {
    text-align: right;
  }
  .help {
    margin: 4px 0 8px;
    max-width: 720px;
  }
  .add {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 12px;
  }
  .add input {
    width: 120px;
  }
  .spacer {
    flex: 1;
  }
</style>
