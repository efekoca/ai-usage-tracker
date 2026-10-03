<script lang="ts">
  import { app, latest } from '../lib/store.svelte'
  import { api, type ModelCompare, type Provider } from '../lib/api'
  import { fmtCompact, fmtInt, fmtMoney, fmtPct, t } from '../lib/i18n.svelte'
  import Segmented from './Segmented.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'
  import Icon from './Icon.svelte'

  let c = $state<ModelCompare | null>(null)
  let failed = $state('')
  $effect(() => {
    void app.tick
    return latest(
      () => api.compareModels($state.snapshot(app.period), $state.snapshot(app.filter)),
      (d) => {
        c = d
        failed = ''
      },
      (e) => {
        c = null
        failed = String(e)
      },
    )
  })

  let provider = $state<'all' | Provider>('all')
  const used = $derived(new Set((c?.actual_by_model ?? []).map(([m]) => m)))
  const rows = $derived((c?.targets ?? []).filter((x) => provider === 'all' || x.provider === provider))
  const top = $derived(Math.max(1e-9, c?.actual_cost_usd ?? 0, ...rows.map((r) => r.cost_usd)))
</script>

<section class="card">
  <div class="head">
    <h2>{t('compare.title')}</h2>
    <AccuracyBadge kind="estimated" compact />
    <span class="spacer"></span>
    <Segmented
      label={t('compare.title')}
      bind:value={provider}
      options={[{ value: 'all', label: t('compare.all') }, { value: 'anthropic', label: t('provider.anthropic') }, { value: 'openai', label: t('provider.openai') }]}
    />
  </div>
  <p class="subtle small lead">{t('compare.lead')}</p>

  {#if failed}
    <p class="muted small">{t('common.error', { e: failed })}</p>
  {:else if !c}
    <p class="muted small">{t('common.loading')}</p>
  {:else if c.basis_events === 0}
    <p class="muted small">{t('compare.none')}</p>
  {:else}
    <div class="actual">
      <span class="muted small">{t('compare.actual')}</span>
      <b class="num">{fmtMoney(c.actual_cost_usd)}</b>
      <span class="subtle small">{t('compare.basis', { n: fmtInt(c.basis_events), t: fmtCompact(c.basis_tokens) })}</span>
    </div>
    <ul class="list" aria-label={t('compare.title')}>
      {#each rows as r (r.model)}
        {@const pct = c.actual_cost_usd > 0 ? (r.delta_usd / c.actual_cost_usd) * 100 : 0}
        {@const same = Math.abs(pct) < 0.5}
        <li class:mine={used.has(r.model)}>
          <span class="name">
            <i class="sw" style="background:var(--{r.provider === 'openai' ? 's1' : 's2'})"></i>
            <span class="mn" title={r.model}>{r.model}</span>
            {#if used.has(r.model)}<span class="tag">{t('compare.used')}</span>{/if}
          </span>
          <span class="bar" aria-hidden="true">
            <span class="fill" style="width:{(r.cost_usd / top) * 100}%;background:var(--{r.provider === 'openai' ? 's1' : 's2'})"></span>
            <span class="mark" style="left:{(c.actual_cost_usd / top) * 100}%"></span>
          </span>
          <span class="num cost">{fmtMoney(r.cost_usd)}</span>
          <span class="num delta" class:down={!same && r.delta_usd < 0} class:up={!same && r.delta_usd > 0}>
            {#if same}{t('compare.same')}
            {:else}
              <Icon name={r.delta_usd < 0 ? 'down' : 'up'} size={12} />
              {r.delta_usd < 0 ? t('compare.cheaper', { v: fmtPct(Math.abs(pct)) }) : t('compare.pricier', { v: fmtPct(pct) })}
            {/if}
          </span>
        </li>
      {/each}
    </ul>
    <p class="subtle small note">
      <span class="key"><i class="mk"></i>{t('compare.actual')}</span>
      {t('compare.caveat')}
      {#if c.excluded_events > 0}{t('compare.excluded', { n: fmtInt(c.excluded_events) })}{/if}
    </p>
  {/if}
</section>

<style>
  section {
    margin-bottom: 16px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .lead {
    margin: 4px 0 14px;
  }
  .actual {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 10px;
  }
  .actual b {
    font-size: 20px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: grid;
    grid-template-columns: minmax(170px, 1.2fr) 2fr 90px 130px;
    gap: 12px;
    align-items: center;
    padding: 6px 0;
    border-bottom: 0.5px solid var(--hairline);
    font-size: 13px;
  }
  li.mine .name {
    font-weight: 600;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: none;
  }
  .tag {
    font-size: 10.5px;
    font-weight: 500;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface-press);
    color: var(--ink-2);
    flex: none;
  }
  .mn {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bar {
    position: relative;
    height: 8px;
    border-radius: 4px;
    background: var(--surface-press);
  }
  .fill {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    border-radius: 4px;
    opacity: 0.85;
  }
  .mark {
    position: absolute;
    top: -3px;
    bottom: -3px;
    width: 2px;
    margin-left: -1px;
    background: var(--ink);
    border-radius: 1px;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .delta {
    display: inline-flex;
    justify-content: flex-end;
    align-items: center;
    gap: 4px;
    color: var(--ink-2);
  }
  .delta.down :global(svg) {
    color: var(--good-ink);
  }
  .delta.up :global(svg) {
    color: var(--serious);
  }
  .note {
    margin: 12px 0 0;
  }
  .key {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-right: 10px;
    color: var(--ink-2);
  }
  .mk {
    display: inline-block;
    width: 2px;
    height: 12px;
    background: var(--ink);
    border-radius: 1px;
  }
  .spacer {
    flex: 1;
  }
  @container main (max-width: 1040px) {
    li {
      grid-template-columns: minmax(150px, 1.3fr) minmax(40px, 1.5fr) 80px 110px;
      gap: 10px;
    }
  }
  @container main (max-width: 700px) {
    li {
      grid-template-columns: 1fr 80px;
    }
    .bar,
    .delta {
      display: none;
    }
  }
</style>
