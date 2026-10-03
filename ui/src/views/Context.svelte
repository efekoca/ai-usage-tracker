<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, type ContextStats } from '../lib/api'
  import { fmtCompact, fmtInt, fmtMoney, fmtPct, t } from '../lib/i18n.svelte'
  import StatTile from '../components/StatTile.svelte'
  import Histogram from '../components/charts/Histogram.svelte'
  import AreaChart from '../components/charts/AreaChart.svelte'
  import Icon from '../components/Icon.svelte'

  let c = $state<ContextStats | null>(null)
  let failed = $state('')
  $effect(() => {
    void app.tick
    api
      .contextStats($state.snapshot(app.period), $state.snapshot(app.filter))
      .then((d) => {
        c = d
        failed = ''
      })
      .catch((e) => (failed = String(e)))
  })

  const k = (n: number) => (n >= 1_000_000 ? `${n / 1_000_000}M` : `${n / 1000}K`)
  // short labels so neighbouring bars never collide
  const bucketLabel = (from: number, to: number | null) =>
    to === null ? `≥${k(from)}` : from === 0 ? `<${k(to)}` : to >= 1_000_000 ? `${k(from)}–${k(to)}` : `${from / 1000}–${k(to)}`
  const bars = $derived(
    (c?.buckets ?? []).map((b) => ({
      key: String(b.from),
      label: bucketLabel(b.from, b.to),
      value: b.requests,
      detail: `${fmtPct((b.requests / Math.max(1, c?.requests ?? 1)) * 100, 1)} · ${fmtMoney(b.cost_usd)}`,
    })),
  )
  const series = [
    { key: 'avg', label: t('context.series.avg'), color: 'var(--s3)' },
    { key: 'p90', label: t('context.series.p90'), color: 'var(--s4, var(--s1))' },
  ]
  const values = $derived((c?.daily ?? []).map((d) => ({ avg: d.avg, p90: d.p90 })))
</script>

<p class="subtle small lead">{t('context.lead')}</p>

{#if failed}
  <div class="banner"><Icon name="warning" size={16} />{t('common.error', { e: failed })}</div>
{:else if !c}
  <p class="muted">{t('common.loading')}</p>
{:else if c.requests === 0}
  <div class="banner"><Icon name="info" size={16} />{t('context.none')}</div>
{:else}
  <section class="tiles card">
    <StatTile hero label={t('context.avg')} value={fmtCompact(c.avg)}>
      <span class="subtle">{fmtInt(c.requests)} {t('context.requests').toLocaleLowerCase()}</span>
    </StatTile>
    <StatTile label={t('context.median')} value={fmtCompact(c.median)} />
    <StatTile label={t('context.p90')} hint={t('context.p90Help')} value={fmtCompact(c.p90)} />
    <StatTile label={t('context.max')} value={fmtCompact(c.max)} />
    <StatTile label={t('context.long')} hint={t('context.longHelp')} value={fmtMoney(c.long_context_extra_usd)}>
      <span class="subtle">{t('context.longCount', { n: fmtInt(c.long_context_requests) })}</span>
    </StatTile>
  </section>

  <div class="grid2">
    <section class="card">
      <h2>{t('context.dist')}</h2>
      <Histogram {bars} format={fmtInt} ariaLabel="{t('context.dist')}: {bars.map((b) => `${b.label} ${b.value}`).join(', ')}" />
    </section>
    <section class="card">
      <h2>{t('context.trend')}</h2>
      {#if c.daily.length > 1}
        <AreaChart dates={c.daily.map((d) => d.date)} {values} {series} format={fmtCompact} ariaLabel={t('context.trend')} />
      {:else}
        <p class="muted small">{t('common.empty')}</p>
      {/if}
    </section>
  </div>

  <section class="card table-card">
    <h2 class="pad">{t('context.byModel')}</h2>
    <div class="scroll">
      <table>
        <thead>
          <tr>
            <th scope="col">{t('common.model')}</th>
            <th scope="col" class="num">{t('context.requests')}</th>
            <th scope="col" class="num">{t('context.avg')}</th>
            <th scope="col" class="num">p90</th>
            <th scope="col" class="num">{t('context.max')}</th>
            <th scope="col" class="num">{t('context.threshold')}</th>
            <th scope="col" class="num">{t('context.over')}</th>
            <th scope="col" class="num">{t('context.extra')}</th>
          </tr>
        </thead>
        <tbody>
          {#each c.by_model as m (m.model)}
            <tr>
              <th scope="row">{m.model}</th>
              <td class="num">{fmtInt(m.requests)}</td>
              <td class="num">{fmtCompact(m.avg)}</td>
              <td class="num">{fmtCompact(m.p90)}</td>
              <td class="num">{fmtCompact(m.max)}</td>
              <td class="num">{m.threshold ? fmtCompact(m.threshold) : t('context.noThreshold')}</td>
              <td class="num">{m.threshold ? fmtInt(m.over_threshold) : '—'}</td>
              <td class="num">{m.threshold ? fmtMoney(m.long_context_extra_usd) : '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <p class="subtle small pad note">{t('context.longHelp')}</p>
  </section>
{/if}

<style>
  .lead {
    margin: 0 0 14px;
    max-width: 760px;
  }
  .tiles {
    display: grid;
    grid-template-columns: 1.3fr repeat(4, 1fr);
    gap: 8px;
    margin-bottom: 16px;
  }
  @media (max-width: 1000px) {
    .tiles {
      grid-template-columns: 1fr 1fr;
    }
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    margin-bottom: 16px;
  }
  @media (max-width: 1000px) {
    .grid2 {
      grid-template-columns: 1fr;
    }
  }
  .grid2 > section {
    min-width: 0;
  }
  h2 {
    margin-bottom: 12px;
  }
  .scroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 8px 12px;
    border-bottom: 0.5px solid var(--hairline);
    text-align: left;
  }
  thead th {
    font-weight: 500;
    color: var(--ink-2);
    font-size: 12px;
    white-space: nowrap;
  }
  tbody th {
    font-weight: 500;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .pad {
    padding: 16px 18px 6px;
  }
  .note {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding-bottom: 14px;
    margin: 0;
  }
</style>
