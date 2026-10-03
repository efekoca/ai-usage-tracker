<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type LimitHistoryView, type PlansFile, type Tip, type Tips } from '../lib/api'
  import { app, latest } from '../lib/store.svelte'
  import { fmtCompact, fmtInt, fmtMoney, fmtPct, has, t, toolLabel } from '../lib/i18n.svelte'
  import Icon from '../components/Icon.svelte'
  import PlanAdvice from '../components/PlanAdvice.svelte'

  let data = $state<Tips | null>(null)
  let history = $state<LimitHistoryView | null>(null)
  let plans = $state<PlansFile | null>(null)
  let failed = $state('')
  onMount(() => {
    api.plans().then((p) => (plans = p)).catch(() => {})
  })
  $effect(() => {
    void app.tick
    return latest(
      () => api.tips($state.snapshot(app.period), $state.snapshot(app.filter)),
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
  $effect(() => {
    void app.tick
    return latest(
      () => api.limitHistory(),
      (h) => (history = h),
      () => (history = null),
    )
  })

  function toolName(tool: string, name: string) {
    const mcp = name.match(/^mcp__(.+?)__(.+)$/)
    if (mcp) return `${mcp[2]} (MCP · ${mcp[1].replaceAll('_', ' ')})`
    return tool === 'codex' && has(`tools.codex.${name}`) ? t(`tools.codex.${name}`) : name
  }
  function text(tip: Tip): { title: string; body: string; icon: string; amount: string | null; share: string | null } {
    const k = `tip.${tip.kind}`
    switch (tip.kind) {
      case 'cache_rebuild':
        return { icon: 'cache', title: t(`${k}.title`), body: t(`${k}.body`, { n: fmtInt(tip.requests), sessions: fmtInt(tip.sessions), tokens: fmtCompact(tip.tokens), usd: fmtMoney(tip.extra_usd) }), amount: fmtMoney(tip.extra_usd), share: fmtPct(tip.cost_share_pct, 1) }
      case 'long_context':
        return { icon: 'context', title: t(`${k}.title`), body: t(`${k}.body`, { n: fmtInt(tip.requests), usd: fmtMoney(tip.extra_usd), models: tip.models.join(', ') }), amount: fmtMoney(tip.extra_usd), share: fmtPct(tip.cost_share_pct, 1) }
      case 'fast_mode':
      case 'residency':
        return { icon: 'clock', title: t(`${k}.title`), body: t(`${k}.body`, { n: fmtInt(tip.requests), usd: fmtMoney(tip.extra_usd) }), amount: fmtMoney(tip.extra_usd), share: fmtPct(tip.cost_share_pct, 1) }
      case 'large_contexts':
        return {
          icon: 'context',
          title: t(`${k}.title`),
          body: t(`${k}.body`, { n: fmtInt(tip.requests), threshold: fmtCompact(tip.threshold), rp: fmtPct(tip.requests_pct, 0), cp: fmtPct(tip.cost_share_pct, 0), usd: fmtMoney(tip.cost_usd) }),
          amount: null,
          share: null,
        }
      case 'tool_errors': {
        const known = tip.known ?? (tip.rate_pct > 0 ? Math.round((tip.failed * 100) / tip.rate_pct) : tip.calls)
        const unknown = Math.max(0, tip.calls - known)
        return {
          icon: 'warning',
          title: t(`${k}.title`, { tool: toolLabel(tip.tool), name: toolName(tip.tool, tip.name) }),
          body:
            t(`${k}.body`, { known: fmtInt(known), rate: fmtPct(tip.rate_pct, 0), failed: fmtInt(tip.failed) }) +
            (unknown > 0 ? ' ' + t(`${k}.unknown`, { n: fmtInt(unknown) }) : ''),
          amount: null,
          share: null,
        }
      }
    }
  }
</script>

<p class="subtle small lead">{t('tips.lead')}</p>

{#if history && history.advice.length}
  <h2 class="section">{t('advice.title')}</h2>
  <div class="advice-grid">
    {#each history.advice as a (a.provider)}
      <PlanAdvice advice={a} {plans} detected={a.provider in history.detected_plans} compact />
    {/each}
  </div>
{/if}

{#if failed}
  <div class="banner"><Icon name="warning" size={16} />{t('common.error', { e: failed })}</div>
{:else if !data}
  <p class="muted">{t('common.loading')}</p>
{:else}
  <h2 class="section">{t('tips.findings')}</h2>
  {#if data.tips.length === 0}
    <div class="card empty">
      <Icon name="check" size={18} />
      <div>
        <p>{t('tips.empty')}</p>
        <p class="subtle small">{t('tips.checked')}</p>
      </div>
    </div>
  {:else}
    <div class="tips">
      {#each data.tips as tip, i (tip.kind + i)}
        {@const x = text(tip)}
        <article class="card tip">
          <div class="head">
            <span class="mark" aria-hidden="true"><Icon name={x.icon} size={16} /></span>
            <h3>{x.title}</h3>
            {#if x.amount}
              <span class="amount">
                <b class="num">{x.amount}</b>
                {#if x.share}<span class="subtle small">{t('tips.share', { pct: x.share })}</span>{/if}
              </span>
            {/if}
          </div>
          <p class="body">{x.body}</p>
          <div class="what">
            <div class="label">{t('tips.what')}</div>
            <p>{t(`tip.${tip.kind}.what`)}</p>
          </div>
          <details>
            <summary>{t('tips.how')}</summary>
            <p class="subtle small">{t(`tip.${tip.kind}.how`)}</p>
          </details>
        </article>
      {/each}
    </div>
  {/if}
  <p class="subtle small basis">{t('tips.basis', { requests: fmtInt(data.requests), calls: fmtInt(data.tool_calls) })} {data.tips.length ? t('tips.checked') : ''}</p>
{/if}

<style>
  .lead {
    max-width: 760px;
    margin: 4px 0 16px;
  }
  .section {
    margin: 8px 0 10px;
  }
  .advice-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(360px, 1fr));
    gap: 16px;
    margin-bottom: 20px;
  }
  .tips {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(420px, 1fr));
    gap: 16px;
  }
  .tip {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 9px;
    flex: none;
    background: var(--surface-hover);
    color: var(--ink-2);
  }
  h3 {
    flex: 1;
    margin: 4px 0 0;
    font-size: 15px;
    font-weight: 600;
    line-height: 1.35;
  }
  .amount {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    text-align: right;
  }
  .amount b {
    font-size: 18px;
    font-weight: 650;
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .body {
    margin: 0;
    line-height: 1.5;
  }
  .what {
    border-left: 2px solid var(--accent);
    padding: 2px 0 2px 12px;
  }
  .what .label {
    font-size: 12px;
    font-weight: 600;
    color: var(--ink-2);
  }
  .what p {
    margin: 2px 0 0;
    font-size: 13px;
    line-height: 1.5;
  }
  details summary {
    cursor: pointer;
    font-size: 12px;
    color: var(--ink-2);
  }
  details p {
    margin: 6px 0 0;
    line-height: 1.5;
  }
  .empty {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    color: var(--good-ink);
  }
  .empty p {
    margin: 0 0 4px;
    color: var(--ink);
  }
  .empty .subtle {
    color: var(--ink-3);
  }
  .basis {
    margin-top: 14px;
    max-width: 760px;
  }
</style>
