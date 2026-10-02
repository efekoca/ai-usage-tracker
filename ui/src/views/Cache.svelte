<script lang="ts">
  // Prompt-cache analysis: hit rate, reuse, net savings, daily trend and breakdowns.
  import { app, toolColor } from '../lib/store.svelte'
  import { clientLabel, fmtCompact, fmtDec, fmtMoney, fmtPct, t, toolLabel } from '../lib/i18n.svelte'
  import type { Group, Tokens } from '../lib/api'
  import StatTile from '../components/StatTile.svelte'
  import AreaChart from '../components/charts/AreaChart.svelte'
  import Segmented from '../components/Segmented.svelte'
  import Icon from '../components/Icon.svelte'

  const r = $derived(app.report)
  const prompt = (k: Tokens) => k.input + k.cache_read + k.cache_write
  const hit = (k: Tokens) => (prompt(k) > 0 ? (k.cache_read / prompt(k)) * 100 : null)
  const reuse = (k: Tokens) => (k.cache_write > 0 ? k.cache_read / k.cache_write : null)

  let breakdown: 'model' | 'client' | 'project' = $state('model')
  const rows = $derived.by((): Group[] => {
    if (!r) return []
    const g = breakdown === 'model' ? r.by_model : breakdown === 'client' ? r.by_client : r.by_project
    return [...g].filter((x) => prompt(x.totals.tokens) > 0).sort((a, b) => prompt(b.totals.tokens) - prompt(a.totals.tokens))
  })
  function label(g: Group): string {
    if (breakdown === 'client') return `${clientLabel(g.key, g.label)} · ${toolLabel(g.key.split(':')[0])}`
    if (breakdown === 'project') return g.key === 'none' ? t('projects.noProject') : g.hidden ? `${t('projects.hidden')} #${g.key}` : g.label
    return g.label
  }

  const days = $derived(r?.daily ?? [])
  const series = [{ key: 'hit', label: t('cache.hitRate'), color: 'var(--s3)' }]
  // days without prompts are gaps, not 0 %
  const values = $derived(days.map((d) => ({ hit: d.prompt_tokens > 0 ? (d.cache_read / d.prompt_tokens) * 100 : NaN })))
  const tk = $derived(r?.totals.tokens)
</script>

{#if !r || !tk}
  <p class="muted">{t('common.loading')}</p>
{:else if prompt(tk) === 0}
  <div class="banner"><Icon name="info" size={16} />{t('common.empty')}</div>
{:else}
  <section class="tiles card">
    <StatTile hero label={t('cache.hitRate')} hint={t('cache.hitRate.help')} value={fmtPct(hit(tk) ?? 0, 1)} current={hit(tk)} previous={hit(r.previous.tokens)} />
    <StatTile label={t('cache.savings')} hint={t('cache.savings.help')} value={fmtMoney(r.totals.cache_savings_usd)} current={r.totals.cache_savings_usd} previous={r.previous.cache_savings_usd} />
    <StatTile label={t('cache.reuse')} hint={t('cache.reuse.help')} value={reuse(tk) !== null ? `${fmtDec(reuse(tk) ?? 0, 1)}×` : '—'} />
    <StatTile label={t('cache.promptTokens')} value={fmtCompact(prompt(tk))}>
      <span class="subtle">{t('cache.uncached')}: {fmtCompact(tk.input)}</span>
    </StatTile>
  </section>

  <div class="grid2">
    <section class="card">
      <h2>{t('cache.mix')}</h2>
      <div class="mixbar" role="img" aria-label="{t('metric.cacheRead')} {fmtPct((tk.cache_read / prompt(tk)) * 100, 1)}, {t('metric.cacheWrite')} {fmtPct((tk.cache_write / prompt(tk)) * 100, 1)}, {t('cache.uncached')} {fmtPct((tk.input / prompt(tk)) * 100, 1)}">
        <span style="flex:{tk.cache_read};background:var(--s3)"></span>
        <span style="flex:{tk.cache_write - tk.cache_write_1h};background:var(--s4)"></span>
        <span style="flex:{tk.cache_write_1h};background:var(--s2)"></span>
        <span style="flex:{tk.input};background:var(--s1)"></span>
      </div>
      <table class="legend">
        <tbody>
          {#each [[t('metric.cacheRead'), tk.cache_read, 'var(--s3)'], [t('cache.write5m'), tk.cache_write - tk.cache_write_1h, 'var(--s4)'], [t('cache.write1h'), tk.cache_write_1h, 'var(--s2)'], [t('cache.uncached'), tk.input, 'var(--s1)']] as [name, v, c] (name)}
            <tr>
              <td><i class="swatch" style="background:{c}"></i></td>
              <td class="name">{name}</td>
              <td class="num">{fmtCompact(v as number)}</td>
              <td class="num subtle">{fmtPct(((v as number) / prompt(tk)) * 100, 1)}</td>
            </tr>
          {/each}
          <tr class="sum">
            <td></td>
            <td class="name">{t('metric.cacheWrite')}</td>
            <td class="num">{fmtCompact(tk.cache_write)}</td>
            <td class="num subtle">{fmtPct((tk.cache_write / prompt(tk)) * 100, 1)}</td>
          </tr>
        </tbody>
      </table>
    </section>
    <section class="card">
      <h2>{t('cache.trend')}</h2>
      <AreaChart dates={days.map((d) => d.date)} {values} {series} format={(v) => fmtPct(v)} height={200} ariaLabel={t('cache.trend')} />
    </section>
  </div>

  <section class="card table-card">
    <div class="th">
      <h2>{t('cache.breakdown')}</h2>
      <span class="spacer"></span>
      <Segmented label={t('cache.breakdown')} bind:value={breakdown} options={[{ value: 'model', label: t('common.model') }, { value: 'client', label: t('common.tool') }, { value: 'project', label: t('common.project') }]} />
    </div>
    <table>
      <thead>
        <tr>
          <th scope="col">{breakdown === 'model' ? t('common.model') : breakdown === 'client' ? t('common.tool') : t('common.project')}</th>
          <th scope="col" class="num">{t('cache.promptTokens')}</th>
          <th scope="col" class="num">{t('metric.cacheRead')}</th>
          <th scope="col" class="num">{t('metric.cacheWrite')}</th>
          <th scope="col" class="num">{t('cache.hitRate')}</th>
          <th scope="col" class="num">{t('cache.reuse')}</th>
          <th scope="col" class="num">{t('cache.savings')}</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as g (g.key)}
          {@const k = g.totals.tokens}
          {@const h = hit(k) ?? 0}
          <tr>
            <th scope="row">
              {#if breakdown === 'client'}<i class="swatch" style="background:{toolColor[g.key.split(':')[0]]}"></i>{/if}
              {label(g)}
            </th>
            <td class="num">{fmtCompact(prompt(k))}</td>
            <td class="num">{fmtCompact(k.cache_read)}</td>
            <td class="num">{fmtCompact(k.cache_write)}</td>
            <td class="num hitcell">
              <span class="minibar" aria-hidden="true"><span style="width:{h}%"></span></span>
              {fmtPct(h, 1)}
            </td>
            <td class="num">{reuse(k) !== null ? `${fmtDec(reuse(k) ?? 0, 1)}×` : '—'}</td>
            <td class="num" class:neg={g.totals.cache_savings_usd < 0}>{g.totals.unpriced_events === g.totals.events ? '—' : fmtMoney(g.totals.cache_savings_usd)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>

  <p class="footnote subtle small"><Icon name="info" size={13} /> {t('cache.method')}</p>
{/if}

<style>
  section {
    margin-bottom: 16px;
  }
  h2 {
    margin-bottom: 12px;
  }
  .tiles {
    display: grid;
    grid-template-columns: 1.3fr 1fr 1fr 1fr;
    gap: 24px;
  }
  @media (max-width: 1000px) {
    .tiles {
      grid-template-columns: 1fr 1fr;
    }
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1.2fr;
    gap: 16px;
  }
  @media (max-width: 960px) {
    .grid2 {
      grid-template-columns: 1fr;
    }
  }
  .mixbar {
    display: flex;
    gap: 2px;
    height: 14px;
    border-radius: 5px;
    overflow: hidden;
    margin: 4px 0 14px;
  }
  .mixbar span {
    min-width: 0;
  }
  .legend {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .legend td {
    padding: 6px 0;
    white-space: nowrap;
  }
  .legend td.num {
    padding-left: 18px;
    width: 1%;
  }
  .legend td:first-child {
    width: 18px;
  }
  .legend .name {
    color: var(--ink-2);
  }
  .legend .sum td {
    border-top: 0.5px solid var(--hairline);
    padding-top: 8px;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    padding-left: 12px;
  }
  .table-card {
    padding: 6px 0;
    overflow-x: auto;
  }
  .th {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 20px 4px;
  }
  .th h2 {
    margin: 0;
  }
  .spacer {
    flex: 1;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .table-card th,
  .table-card td {
    padding: 8px 20px;
    text-align: left;
    border-bottom: 0.5px solid var(--hairline);
    white-space: nowrap;
  }
  .table-card .num {
    text-align: right;
  }
  thead th {
    color: var(--ink-2);
    font-weight: 500;
    font-size: 12px;
  }
  tbody th {
    font-weight: 500;
  }
  tbody th .swatch {
    display: inline-block;
    margin-right: 6px;
  }
  tbody tr:last-child td,
  tbody tr:last-child th {
    border-bottom: 0;
  }
  .hitcell {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }
  .minibar {
    width: 64px;
    height: 5px;
    border-radius: 3px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .minibar span {
    display: block;
    height: 100%;
    background: var(--s3);
  }
  .neg {
    color: var(--bad-ink);
  }
  .footnote {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    max-width: 820px;
  }
</style>
