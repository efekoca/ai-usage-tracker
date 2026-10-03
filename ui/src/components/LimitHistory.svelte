<script lang="ts">
  // Limits page: plan recommendation per provider, then the history of every five-hour and
  // weekly window the readings cover.
  import { api, type LimitHistoryView, type PlansFile, type Provider } from '../lib/api'
  import { app } from '../lib/store.svelte'
  import { fmtPct, t } from '../lib/i18n.svelte'
  import PlanAdvice from './PlanAdvice.svelte'
  import WindowHistory from './charts/WindowHistory.svelte'

  let { plans }: { plans: PlansFile | null } = $props()

  let view = $state<LimitHistoryView | null>(null)
  $effect(() => {
    void app.tick
    void app.settings?.plans
    api.limitHistory().then((v) => (view = v)).catch(() => (view = null))
  })

  const DAY = 864e5
  const providers = $derived([...new Set((view?.history.series ?? []).map((s) => s.provider))].sort() as Provider[])
  const series = (p: Provider, w: string) => view?.history.series.find((s) => s.provider === p && s.window === w)
  function summary(p: Provider, w: string, from: number) {
    const ws = (series(p, w)?.windows ?? []).filter((x) => (x.resets_at_ms ?? x.last_ms) >= from)
    if (!ws.length) return ''
    return t('history.summary', { n: ws.length, full: ws.filter((x) => x.full).length, peak: fmtPct(Math.max(...ws.map((x) => x.peak_pct))) })
  }
</script>

{#if view}
  {#if view.advice.length}
    <section class="block">
      <h2>{t('advice.title')}</h2>
      <p class="subtle small lead">{t('advice.lead')}</p>
      <div class="advice-grid">
        {#each view.advice as a (a.provider)}
          <PlanAdvice advice={a} {plans} detected={a.provider in view.detected_plans} />
        {/each}
      </div>
    </section>
  {/if}

  <section class="card block">
    <h2>{t('history.title')}</h2>
    <p class="subtle small lead">{t('history.lead')}</p>
    {#if providers.length === 0}
      <p class="muted">{t('history.empty')}</p>
    {:else}
      <div class="legend" aria-hidden="true">
        <span><i class="sw complete"></i>{t('history.legend.complete')}</span>
        <span><i class="sw partial"></i>{t('history.legend.partial')}</span>
        <span><i class="sw full"></i>{t('history.legend.full')}</span>
        <span><i class="sw running"></i>{t('history.legend.running')}</span>
      </div>
      {#each providers as p (p)}
        {@const to = view.history.to_ms}
        <div class="prov">
          <h3>{t(`provider.${p}`)}</h3>
          <div class="charts">
            {#each [{ w: 'five_hour', from: to - 28 * DAY, label: t('history.five') }, { w: 'seven_day', from: view.history.from_ms, label: t('history.week') }] as c (c.w)}
              <div class="chart">
                <div class="small muted">{c.label}</div>
                {#if series(p, c.w)}
                  <WindowHistory windows={series(p, c.w)!.windows} fromMs={c.from} toMs={to} ariaLabel="{t(`provider.${p}`)} · {c.label}: {summary(p, c.w, c.from)}" />
                  <div class="small subtle sum">{summary(p, c.w, c.from)}</div>
                {:else}
                  <p class="subtle small empty">{t('history.empty')}</p>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/each}
    {/if}
  </section>
{/if}

<style>
  .block {
    margin-bottom: 16px;
  }
  .lead {
    margin: 4px 0 12px;
    max-width: 760px;
  }
  .advice-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(360px, 1fr));
    gap: 16px;
  }
  .prov + .prov {
    border-top: 0.5px solid var(--hairline);
    margin-top: 14px;
    padding-top: 14px;
  }
  h3 {
    font-size: 13px;
    font-weight: 600;
    margin: 0 0 8px;
  }
  .charts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 24px;
  }
  @media (max-width: 1000px) {
    .charts {
      grid-template-columns: 1fr;
    }
  }
  .chart {
    min-width: 0;
  }
  .sum {
    margin-top: 4px;
  }
  .empty {
    padding: 24px 0;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    font-size: 12px;
    color: var(--ink-2);
    margin-bottom: 12px;
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .sw {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    display: inline-block;
    background: var(--s1);
  }
  .sw.partial {
    background: repeating-linear-gradient(45deg, var(--s1) 0 2px, var(--surface) 2px 4px);
    box-shadow: inset 0 0 0 1px var(--s1);
  }
  .sw.full {
    background: var(--critical);
  }
  .sw.running {
    opacity: 0.55;
  }
</style>
