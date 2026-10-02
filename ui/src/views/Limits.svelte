<script lang="ts">
  import { onMount } from 'svelte'
  import { app, saveSettings } from '../lib/store.svelte'
  import { api, type PlansFile, type Provider, type Threshold } from '../lib/api'
  import { fmtCompact, fmtMoney, fmtPct, i18n, t, windowLabel } from '../lib/i18n.svelte'
  import LimitMeter from '../components/LimitMeter.svelte'
  import AccuracyBadge from '../components/AccuracyBadge.svelte'
  import Icon from '../components/Icon.svelte'
  import Segmented from '../components/Segmented.svelte'

  let plans: PlansFile | null = $state(null)
  onMount(async () => {
    plans = await api.plans()
  })

  const providers = $derived.by(() => {
    const s = new Set<Provider>()
    for (const src of app.settings?.enabled_sources ?? []) {
      if (src === 'codex') s.add('openai')
      else if (src !== 'chatgpt_desktop') s.add('anthropic')
    }
    return [...s]
  })

  function setPlan(p: Provider, id: string) {
    const next = { ...(app.settings?.plans ?? {}) }
    if (id) next[p] = id
    else delete next[p]
    saveSettings({ plans: next })
  }

  let draft: Threshold = $state({ provider: 'anthropic', window: 'five_hour', tokens: null, cost_usd: null })
  let draftKind: 'cost' | 'tokens' = $state('cost')
  let draftValue = $state('')

  function addThreshold() {
    const v = Number(draftValue.replace(',', '.'))
    if (!(v > 0)) return
    const th: Threshold = { provider: draft.provider, window: draft.window, tokens: draftKind === 'tokens' ? Math.round(v) : null, cost_usd: draftKind === 'cost' ? v : null }
    const rest = (app.settings?.thresholds ?? []).filter((x) => !(x.provider === th.provider && x.window === th.window))
    saveSettings({ thresholds: [...rest, th] }).then(() => api.limits().then((l) => (app.limits = l)))
    draftValue = ''
  }
  function removeThreshold(i: number) {
    const list = [...(app.settings?.thresholds ?? [])]
    list.splice(i, 1)
    saveSettings({ thresholds: list }).then(() => api.limits().then((l) => (app.limits = l)))
  }
  const planNote = (p: Provider) => plans?.providers[p]?.plans.find((x) => x.id === app.settings?.plans[p])
  function setPrice(p: Provider, raw: string) {
    const v = Number(raw.replace(',', '.'))
    const next = { ...(app.settings?.plan_prices ?? {}) }
    if (v > 0) next[p] = v
    else delete next[p]
    saveSettings({ plan_prices: next })
  }
</script>

<header class="bar">
  <h1>{t('limits.title')}</h1>
  <span class="spacer"></span>
  <span class="subtle small" title={t('limits.mode.help')}>{t('limits.mode')}</span>
  <Segmented label={t('limits.mode')} value={app.settings?.limit_display ?? 'used'} options={[{ value: 'used', label: t('limits.mode.used') }, { value: 'remaining', label: t('limits.mode.remaining') }]} onchange={(v) => saveSettings({ limit_display: v })} />
</header>

{#if plans}
  <section class="card plans">
    {#each providers as p (p)}
      <div class="plan-row">
        <label for="plan-{p}"><b>{plans.providers[p].label}</b> · {t('limits.plan')}</label>
        <select id="plan-{p}" class="field" value={app.settings?.plans[p] ?? ''} onchange={(e) => setPlan(p, e.currentTarget.value)}>
          <option value="">{t('onb.plan.none')}</option>
          {#each plans.providers[p].plans as pl (pl.id)}<option value={pl.id}>{pl.name}{pl.relative ? ` (${pl.relative})` : ''}</option>{/each}
        </select>
        {#if planNote(p)}
          {@const pl = planNote(p)!}
          {#if pl.id !== 'api' && pl.monthly_usd !== 0}
            {@const own = app.settings?.plan_prices?.[p]}
            <label class="price">
              <span class="subtle small">{t('value.price')}</span>
              <input class="field" inputmode="decimal" value={own ?? ''} placeholder={typeof pl.monthly_usd === 'number' ? `${pl.monthly_usd} (${t('value.listPrice')})` : '—'} onchange={(e) => setPrice(p, e.currentTarget.value)} title={pl.price_note?.[i18n.lang] ?? ''} />
            </label>
          {/if}
          <span class="subtle small">{pl.note ? pl.note[i18n.lang] : ''}</span>
          <button class="btn ghost small" onclick={() => api.openUrl(pl.source)}><Icon name="external" size={13} />{t('settings.pricing.sources')}</button>
        {/if}
      </div>
    {/each}
    <p class="subtle small">{plans.notes[i18n.lang]} {plans.providers.anthropic.shared_pool_note?.[i18n.lang] ?? ''}</p>
  </section>
{/if}

{#if app.limits.length === 0}
  <div class="banner"><Icon name="info" size={16} />{t('limits.none')}</div>
{/if}

<div class="cards">
  {#each app.limits as l (l.provider + l.limit_id + l.window + l.source)}
    <section class="card limit">
      <div class="who">
        <span class="prov">{t(`provider.${l.provider}`)}</span>
        {#if l.plan}<span class="pill">{l.plan}</span>{/if}
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
      {#if l.projects.length}
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
          </ul>
        </div>
      {/if}
    </section>
  {/each}
</div>

<section class="card">
  <h2>{t('limits.thresholds')}</h2>
  <p class="subtle small help">{t('limits.thresholds.help')}</p>
  {#each app.settings?.thresholds ?? [] as th, i (th.provider + th.window)}
    <div class="list-row">
      <span>{t(`provider.${th.provider}`)} · {windowLabel(th.window)}</span>
      <span class="spacer"></span>
      <span class="num">{th.cost_usd ? fmtMoney(th.cost_usd) : `${fmtCompact(th.tokens ?? 0)} ${t('metric.tokens')}`}</span>
      <button class="btn ghost" aria-label={t('common.remove')} onclick={() => removeThreshold(i)}><Icon name="trash" size={15} /></button>
    </div>
  {/each}
  <div class="add">
    <select class="field" bind:value={draft.provider} aria-label={t('common.tool')}>
      {#each providers as p (p)}<option value={p}>{t(`provider.${p}`)}</option>{/each}
    </select>
    <select class="field" bind:value={draft.window} aria-label={t('limits.title')}>
      <option value="five_hour">{windowLabel('five_hour')}</option>
      <option value="seven_day">{windowLabel('seven_day')}</option>
    </select>
    <select class="field" bind:value={draftKind} aria-label={t('metric.cost')}>
      <option value="cost">{t('metric.cost')} (USD)</option>
      <option value="tokens">{t('metric.tokens')}</option>
    </select>
    <input class="field" inputmode="decimal" bind:value={draftValue} placeholder={draftKind === 'cost' ? '50' : '5000000'} aria-label="value" />
    <button class="btn" onclick={addThreshold}><Icon name="plus" size={14} />{t('limits.addThreshold')}</button>
  </div>
</section>

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
  }
  .plan-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .plan-row label {
    min-width: 170px;
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
