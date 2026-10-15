<script lang="ts">
  import { api, type PromptShare, type SubagentCache, type Tokens, type Tool, type Totals } from '../lib/api'
  import { app, latest, toolColor } from '../lib/store.svelte'
  import { fmtCompact, fmtDec, fmtInt, fmtMoney, fmtPct, has, t, toolLabel } from '../lib/i18n.svelte'
  import StatTile from './StatTile.svelte'
  import BarValue from './BarValue.svelte'
  import Icon from './Icon.svelte'

  // smaller samples and gaps are noise
  const MIN_AFTER = 5
  const GAP_PTS = 5

  let data = $state<SubagentCache | null>(null)
  let failed = $state('')
  $effect(() => {
    void app.tick
    return latest(
      () => api.subagentCache($state.snapshot(app.period), $state.snapshot(app.filter)),
      (d) => {
        data = d
        failed = ''
      },
      (e) => {
        data = null
        failed = String(e)
      },
    )
  })

  const prompt = (k: Tokens) => k.input + k.cache_read + k.cache_write
  const hit = (k: Tokens) => (prompt(k) > 0 ? (k.cache_read / prompt(k)) * 100 : null)
  const shareHit = (s: PromptShare) => (s.prompt > 0 ? (s.cache_read / s.prompt) * 100 : null)
  const reuse = (x: Totals) => (x.cache_write_with_reads > 0 ? x.cache_read_with_writes / x.cache_write_with_reads : null)
  const oneHour = (k: Tokens) => (k.cache_write > 0 ? (k.cache_write_1h / k.cache_write) * 100 : null)
  const priced = (x: Totals) => x.events > 0 && x.unpriced_events < x.events
  const agentLabel = (a: string | null) => (a === null ? t('agent.main') : has(`agent.${a}`) ? t(`agent.${a}`) : a)
  const pct = (v: number | null, digits = 1) => (v === null ? '—' : fmtPct(v, digits))
  const times = (v: number | null) => (v === null ? '—' : `${fmtDec(v, 1)}×`)

  const used = $derived(!!data && data.subagents.events > 0)
  const mainHit = $derived(data ? hit(data.main.tokens) : null)
  const subHit = $derived(data ? hit(data.subagents.tokens) : null)
  const writeCost = $derived(data ? data.main.cost.cache_write + data.subagents.cost.cache_write : 0)
  const writeShare = $derived(data && writeCost > 0 ? (data.subagents.cost.cache_write / writeCost) * 100 : null)
  const perRun = $derived(data && data.runs > 0 ? data.subagents.events / data.runs : null)
  const writePerRun = $derived(data && data.runs > 0 ? data.subagents.tokens.cache_write / data.runs : null)

  type Finding = { tone: 'good' | 'warn' | 'info'; text: string }
  const findings = $derived.by((): Finding[] => {
    if (!data || !used) return []
    const out: Finding[] = []
    const after = shareHit(data.main_after_run)
    const other = shareHit(data.main_other)
    if (data.main_after_run.requests < MIN_AFTER || after === null || other === null) {
      out.push({ tone: 'info', text: t('subcache.f.mainFew', { n: fmtInt(data.main_after_run.requests), min: MIN_AFTER }) })
    } else {
      const args = { after: fmtPct(after, 1), other: fmtPct(other, 1) }
      out.push(after < other - GAP_PTS ? { tone: 'warn', text: t('subcache.f.mainDrop', args) } : { tone: 'good', text: t('subcache.f.mainOk', args) })
    }
    if (subHit !== null && mainHit !== null) {
      const r = reuse(data.subagents)
      if (subHit < mainHit - GAP_PTS && perRun !== null) {
        const args = { pts: fmtDec(mainHit - subHit, 1), req: fmtDec(perRun, 1), reuse: r === null ? '' : fmtDec(r, 1) }
        out.push({ tone: 'warn', text: t(r === null ? 'subcache.f.subLowerNoReuse' : 'subcache.f.subLower', args) })
      } else {
        out.push({ tone: 'good', text: t('subcache.f.subClose', { sub: fmtPct(subHit, 1), main: fmtPct(mainHit, 1) }) })
      }
    }
    if (writeShare !== null && priced(data.subagents)) {
      out.push({ tone: 'info', text: t('subcache.f.writeShare', { pct: fmtPct(writeShare, 1), usd: fmtMoney(data.subagents.cost.cache_write) }) })
    }
    const subTtl = oneHour(data.subagents.tokens)
    const mainTtl = oneHour(data.main.tokens)
    if (subTtl !== null && mainTtl !== null && subTtl < 50 && mainTtl >= 50) out.push({ tone: 'info', text: t('subcache.f.ttl') })
    const warm = shareHit(data.first)
    if (warm !== null && data.first.requests > 0) out.push({ tone: 'info', text: t('subcache.f.warm', { pct: fmtPct(warm, 1) }) })
    return out
  })

  const tools = $derived([...new Set((data?.rows ?? []).filter((r) => r.agent !== null).map((r) => r.tool))] as Tool[])
  const icon = { good: 'check', warn: 'warning', info: 'info' } as const
</script>

{#if failed}
  <section class="card"><p class="muted">{failed}</p></section>
{:else if data}
  <section class="card intro">
    <header>
      <h2>{t('subcache.title')}</h2>
      <p class="subtle small lead">{t('subcache.lead')}</p>
    </header>

    {#if !used}
      <p class="muted empty">{t('subcache.none')}</p>
    {:else}
      <div class="tiles">
        <StatTile label={t('subcache.subHit')} hint={t('cache.hitRate.help')} value={pct(subHit)} current={subHit} previous={shareHit(data.previous_subagents)} />
        <StatTile label={t('subcache.mainHit')} hint={t('cache.hitRate.help')} value={pct(mainHit)} current={mainHit} previous={shareHit(data.previous_main)} />
        <StatTile label={t('subcache.writeShare')} hint={t('subcache.writeShare.help')} value={priced(data.subagents) ? pct(writeShare) : '—'}>
          {#if priced(data.subagents)}<span class="subtle">{fmtMoney(data.subagents.cost.cache_write)}</span>{/if}
        </StatTile>
        <StatTile label={t('subcache.runs')} value={fmtInt(data.runs)}>
          <span class="subtle">{t('subcache.runsIn', { n: fmtInt(data.sessions) })}</span>
        </StatTile>
      </div>
    {/if}
  </section>

  {#if used}
    <div class="grid2">
      <section class="card table-card">
        <header class="head"><h2>{t('subcache.compare')}</h2></header>
        <div class="scroll">
          <table class="compare">
            <thead>
              <tr>
                <th scope="col"><span class="sr">{t('subcache.metric')}</span></th>
                <th scope="col" class="num">{t('agent.main')}</th>
                <th scope="col" class="num">{t('agents.title')}</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <th scope="row" title={t('cache.hitRate.help')}>{t('cache.hitRate')}</th>
                <td class="num"><BarValue pct={mainHit ?? 0} tone="teal" text={pct(mainHit)} /></td>
                <td class="num"><BarValue pct={subHit ?? 0} tone="teal" text={pct(subHit)} /></td>
              </tr>
              <tr>
                <th scope="row" title={t('cache.reuse.help')}>{t('cache.reuse')}</th>
                <td class="num">{times(reuse(data.main))}</td>
                <td class="num">{times(reuse(data.subagents))}</td>
              </tr>
              <tr>
                <th scope="row">{t('cache.promptTokens')}</th>
                <td class="num">{fmtCompact(prompt(data.main.tokens))}</td>
                <td class="num">{fmtCompact(prompt(data.subagents.tokens))}</td>
              </tr>
              <tr>
                <th scope="row">{t('metric.cacheWrite')}</th>
                <td class="num">{fmtCompact(data.main.tokens.cache_write)}</td>
                <td class="num">{fmtCompact(data.subagents.tokens.cache_write)}</td>
              </tr>
              <tr>
                <th scope="row">{t('subcache.row.write1h')}</th>
                <td class="num">{pct(oneHour(data.main.tokens), 0)}</td>
                <td class="num">{pct(oneHour(data.subagents.tokens), 0)}</td>
              </tr>
              <tr>
                <th scope="row">{t('subcache.row.writeCost')}</th>
                <td class="num">{priced(data.main) ? fmtMoney(data.main.cost.cache_write) : '—'}</td>
                <td class="num">{priced(data.subagents) ? fmtMoney(data.subagents.cost.cache_write) : '—'}</td>
              </tr>
              <tr>
                <th scope="row" title={t('cache.savings.help')}>{t('cache.savings')}</th>
                <td class="num" class:neg={data.main.cache_savings_usd < 0}>{priced(data.main) ? fmtMoney(data.main.cache_savings_usd) : '—'}</td>
                <td class="num" class:neg={data.subagents.cache_savings_usd < 0}>{priced(data.subagents) ? fmtMoney(data.subagents.cache_savings_usd) : '—'}</td>
              </tr>
              <tr>
                <th scope="row">{t('subcache.row.perRun')}</th>
                <td class="num subtle">—</td>
                <td class="num">{perRun === null ? '—' : fmtDec(perRun, 1)}</td>
              </tr>
              <tr>
                <th scope="row">{t('subcache.row.writePerRun')}</th>
                <td class="num subtle">—</td>
                <td class="num">{writePerRun === null ? '—' : fmtCompact(writePerRun)}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section class="card">
        <h2>{t('subcache.findings')}</h2>
        <ul class="findings">
          {#each findings as f (f.text)}
            <li class={f.tone}><span class="badge"><Icon name={icon[f.tone]} size={14} /></span><span>{f.text}</span></li>
          {/each}
        </ul>
      </section>
    </div>

    <section class="card table-card">
      <header class="head"><h2>{t('subcache.byAgent')}</h2></header>
      <div class="scroll">
        <table class="agents">
          <thead>
            <tr>
              <th scope="col">{t('agents.col.agent')}</th>
              <th scope="col" class="num" title={t('subcache.perRunHelp')}>{t('agents.col.runs')}</th>
              <th scope="col" class="num">{t('cache.promptTokens')}</th>
              <th scope="col" class="num">{t('subcache.hit')}</th>
              <th scope="col" class="num" title={t('subcache.col.first.help')}>{t('subcache.col.first')}</th>
              <th scope="col" class="num" title={t('cache.reuse.help')}>{t('cache.reuse')}</th>
              <th scope="col" class="num" title={t('subcache.perRunHelp')}>{t('subcache.row.writeCost')}</th>
            </tr>
          </thead>
          {#each tools as tool (tool)}
            <tbody>
              <tr class="group">
                <th scope="rowgroup" colspan="7"><i class="dot" style="background:{toolColor[tool]}"></i>{toolLabel(tool)}</th>
              </tr>
              {#each data.rows.filter((r) => r.tool === tool) as r (r.agent ?? '')}
                {@const h = hit(r.totals.tokens)}
                {@const sub = r.agent !== null && r.runs > 0}
                <tr>
                  <th scope="row" class:main={r.agent === null}>{agentLabel(r.agent)}</th>
                  <td class="num">
                    {#if sub}
                      <span class="v">{fmtInt(r.runs)}</span>
                      <span class="sub">{t('subcache.reqPerRun', { n: fmtDec(r.totals.events / r.runs, 1) })}</span>
                    {:else}<span class="subtle">—</span>{/if}
                  </td>
                  <td class="num">{fmtCompact(prompt(r.totals.tokens))}</td>
                  <td class="num"><BarValue pct={h ?? 0} tone="teal" text={pct(h)} /></td>
                  <td class="num">{#if sub}{pct(shareHit(r.first))}{:else}<span class="subtle">—</span>{/if}</td>
                  <td class="num">{times(reuse(r.totals))}</td>
                  <td class="num">
                    <span class="v">{priced(r.totals) ? fmtMoney(r.totals.cost.cache_write) : '—'}</span>
                    {#if sub}<span class="sub">{t('subcache.writePerRun', { n: fmtCompact(r.totals.tokens.cache_write / r.runs) })}</span>{/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          {/each}
        </table>
      </div>
      <footer class="foot subtle small"><Icon name="info" size={13} /><span>{t('subcache.method')}</span></footer>
    </section>
  {/if}
{/if}

<style>
  section {
    margin-bottom: 16px;
  }
  h2 {
    margin: 0;
  }
  .intro header {
    margin-bottom: 18px;
  }
  .lead {
    margin: 6px 0 0;
    max-width: 760px;
    line-height: 1.5;
  }
  .empty {
    margin: 0;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 24px;
    padding-top: 16px;
    border-top: 0.5px solid var(--hairline);
  }
  @container main (max-width: 940px) {
    .tiles {
      grid-template-columns: 1fr 1fr;
    }
  }
  .grid2 {
    display: grid;
    grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr);
    gap: 16px;
    margin-bottom: 16px;
  }
  .grid2 > section {
    min-width: 0;
    margin-bottom: 0;
  }
  @container main (max-width: 940px) {
    .grid2 {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .table-card {
    padding: 0;
    overflow: hidden;
  }
  .head {
    padding: 18px 20px 10px;
  }
  .scroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .table-card th,
  .table-card td {
    padding: 10px 14px;
    text-align: left;
    border-top: 0.5px solid var(--hairline);
    white-space: nowrap;
    vertical-align: middle;
  }
  .table-card th:first-child,
  .table-card td:first-child {
    padding-left: 20px;
  }
  .table-card th:last-child,
  .table-card td:last-child {
    padding-right: 20px;
  }
  .table-card .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  thead th {
    color: var(--ink-3);
    font-weight: 500;
    font-size: 12px;
    vertical-align: bottom;
  }
  tbody th {
    font-weight: 500;
  }
  .compare tbody th {
    color: var(--ink-2);
    font-weight: 400;
  }
  tbody th.main {
    font-weight: 600;
  }
  tr.group th {
    background: var(--surface-2);
    font-size: 12px;
    font-weight: 600;
    color: var(--ink-2);
    padding-top: 8px;
    padding-bottom: 8px;
  }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-right: 8px;
  }
  .v,
  .sub {
    display: block;
  }
  .sub {
    margin-top: 2px;
    font-size: 11px;
    color: var(--ink-3);
  }
  .neg {
    color: var(--bad-ink);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  .findings {
    list-style: none;
    margin: 14px 0 0;
    padding: 0;
    display: grid;
    gap: 10px;
    font-size: 13px;
    line-height: 1.5;
  }
  .findings li {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    padding: 12px 14px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }
  .badge {
    flex: none;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--surface);
    border: 0.5px solid var(--hairline);
  }
  .good .badge {
    color: var(--good-ink);
  }
  .warn .badge {
    color: var(--warning);
  }
  .info .badge {
    color: var(--ink-3);
  }
  .foot {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 14px 20px 16px;
    border-top: 0.5px solid var(--hairline);
    line-height: 1.5;
  }
  .foot :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  @container main (max-width: 1040px) {
    .table-card th,
    .table-card td {
      padding-left: 10px;
      padding-right: 10px;
    }
  }
</style>
