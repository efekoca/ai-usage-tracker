<script lang="ts">
  import { app, toolColor } from '../lib/store.svelte'
  import { clientLabel, fmtCompact, fmtMoney, fmtPct, t, toolLabel } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import BarList from '../components/charts/BarList.svelte'
  import Composition from '../components/charts/Composition.svelte'
  import HourHeatmap from '../components/charts/HourHeatmap.svelte'
  import Icon from '../components/Icon.svelte'

  let metric: 'tokens' | 'cost' = $state(app.settings?.primary_metric ?? 'tokens')
  const r = $derived(app.report)
  const fmt = $derived(metric === 'tokens' ? fmtCompact : (v: number) => fmtMoney(v))
  const total = $derived(Math.max(1e-9, metric === 'tokens' ? (r?.totals.total_tokens ?? 0) : (r?.totals.cost_usd ?? 0)))
</script>

{#if !r}
  <p class="muted">{t('common.loading')}</p>
{:else}
  <div class="head">
    <Segmented label={t('metric.tokens')} options={[{ value: 'tokens', label: t('metric.tokens') }, { value: 'cost', label: t('metric.cost') }]} bind:value={metric} />
  </div>

  <section class="card table-card">
    <h2 class="pad">{t('breakdown.models')}</h2>
    <table>
      <thead>
        <tr>
          <th scope="col">{t('common.model')}</th>
          <th scope="col" class="num">{t('metric.input')}</th>
          <th scope="col" class="num">{t('metric.output')}</th>
          <th scope="col" class="num">{t('metric.cacheRead')}</th>
          <th scope="col" class="num">{t('metric.cacheWrite')}</th>
          <th scope="col" class="num">{t('metric.apiEq')}</th>
          <th scope="col" class="num">{t('breakdown.share')}</th>
        </tr>
      </thead>
      <tbody>
        {#each r.by_model as g (g.key)}
          {@const v = metric === 'tokens' ? g.totals.total_tokens : g.totals.cost_usd}
          <tr>
            <th scope="row">{g.label}</th>
            <td class="num">{fmtCompact(g.totals.tokens.input)}</td>
            <td class="num">{fmtCompact(g.totals.tokens.output)}</td>
            <td class="num">{fmtCompact(g.totals.tokens.cache_read)}</td>
            <td class="num">{fmtCompact(g.totals.tokens.cache_write)}</td>
            <td class="num">
              {#if g.totals.unpriced_events > 0}
                <span class="unpriced" title={t('overview.unpricedAction')}><Icon name="warning" size={13} /> —</span>
              {:else}
                {fmtMoney(g.totals.cost_usd)}
              {/if}
            </td>
            <td class="num share">
              <span class="minibar" aria-hidden="true"><span style="width:{(v / total) * 100}%"></span></span>
              {fmtPct((v / total) * 100, 1)}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>

  <div class="grid2">
    <section class="card">
      <h2>{t('breakdown.tools')}</h2>
      <BarList
        ariaLabel={t('breakdown.tools')}
        items={r.by_client.map((g) => {
          const tool = g.key.split(':')[0]
          return { key: g.key, label: clientLabel(g.key, g.label), sub: toolLabel(tool), value: metric === 'tokens' ? g.totals.total_tokens : g.totals.cost_usd, color: toolColor[tool] }
        })}
        format={fmt}
      />
    </section>
    <section class="card">
      <h2>{t('breakdown.categories')}</h2>
      <Composition tokens={r.totals.tokens} />
      <table class="costs">
        <tbody>
          <tr><td>{t('metric.input')}</td><td class="num">{fmtMoney(r.totals.cost.input)}</td></tr>
          <tr><td>{t('metric.output')}</td><td class="num">{fmtMoney(r.totals.cost.output)}</td></tr>
          <tr><td>{t('metric.cacheRead')}</td><td class="num">{fmtMoney(r.totals.cost.cache_read)}</td></tr>
          <tr><td>{t('metric.cacheWrite')}</td><td class="num">{fmtMoney(r.totals.cost.cache_write)}</td></tr>
          {#if r.totals.cost.web_search > 0}<tr><td>Web search</td><td class="num">{fmtMoney(r.totals.cost.web_search)}</td></tr>{/if}
          <tr class="sum"><td>{t('metric.apiEq')}</td><td class="num">{fmtMoney(r.totals.cost_usd)}</td></tr>
        </tbody>
      </table>
    </section>
  </div>

  <section class="card">
    <h2>{t('breakdown.hours')}</h2>
    <HourHeatmap grid={r.heatmap} format={fmtCompact} ariaLabel={t('breakdown.hours')} />
  </section>
{/if}

<style>
  .head {
    margin-bottom: 16px;
  }
  section {
    margin-bottom: 16px;
  }
  h2 {
    margin-bottom: 12px;
  }
  .pad {
    padding: 12px 20px 0;
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }
  @media (max-width: 960px) {
    .grid2 {
      grid-template-columns: 1fr;
    }
  }
  .table-card {
    padding: 6px 0;
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 8px 20px;
    text-align: left;
    border-bottom: 0.5px solid var(--hairline);
    white-space: nowrap;
  }
  thead th {
    color: var(--ink-2);
    font-weight: 500;
    font-size: 12px;
  }
  tbody th {
    font-weight: 500;
  }
  tbody tr:last-child td,
  tbody tr:last-child th {
    border-bottom: 0;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .share {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }
  .minibar {
    width: 60px;
    height: 5px;
    border-radius: 3px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .minibar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .unpriced {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    color: var(--ink-2);
  }
  .costs {
    margin-top: 14px;
  }
  .costs td {
    padding: 5px 0;
    color: var(--ink-2);
  }
  .costs .sum td {
    color: var(--ink);
    font-weight: 600;
  }
</style>
