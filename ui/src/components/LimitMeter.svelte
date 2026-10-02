<script lang="ts">
  // A plan-limit meter. Severity uses the reserved status colors and always ships with an
  // icon + label; stale/reset readings are shown as unknown instead of a misleading bar.
  import Icon from './Icon.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'
  import { fmtCompact, fmtDec, fmtDuration, fmtLimit, fmtTime, fmtWhen, limitShown, t, windowLabel } from '../lib/i18n.svelte'
  import type { Accuracy, Forecast, LimitState, Provider } from '../lib/api'
  import { app } from '../lib/store.svelte'

  let {
    window: win,
    used,
    state: lstate,
    accuracy,
    resetsAt = null,
    observedMs = null,
    source = '',
    title = '',
    compact = false,
    sinceTokens = 0,
    provider = null,
    forecast = null,
  }: {
    window: string
    used: number | null
    state: LimitState
    accuracy: Accuracy
    resetsAt?: number | null
    observedMs?: number | null
    source?: string
    title?: string
    compact?: boolean
    /** tokens used after the reading ('behind' only) */
    sinceTokens?: number
    provider?: Provider | null
    /** pace of the current window (fresh readings only) */
    forecast?: Forecast | null
  } = $props()

  let now = $state(Date.now())
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 30_000)
    return () => clearInterval(id)
  })

  // 'behind': the provider was used after this reading, so the current value is unknown (only
  // that it is at least the reading). It is shown as the last reading, never as the current value.
  const known = $derived(lstate === 'fresh' && used !== null)
  const behind = $derived(lstate === 'behind' && used !== null)
  const pct = $derived(known ? Math.max(0, Math.min(100, used as number)) : 0)
  const level = $derived(behind ? 'outdated' : !known ? 'unknown' : pct >= 100 ? 'full' : pct >= 90 ? 'high' : pct >= 70 ? 'warn' : 'ok')
  const icon = $derived(level === 'ok' ? 'check' : level === 'unknown' || level === 'outdated' ? 'clock' : 'warning')
  // severity always follows usage; only the number and the bar follow the chosen reading
  const mode = $derived(app.settings?.limit_display ?? 'used')
  const shown = $derived(known ? limitShown(pct, mode) : 0)
</script>

<div class="meter" class:compact>
  <div class="head">
    <span class="name">{title || windowLabel(win)}</span>
    <span class="spacer"></span>
    {#if known}
      <span class="pct num">{fmtLimit(pct, mode)}</span>
    {:else if behind}
      <span class="pct subtle" title={t('limits.state.behind.help')}>?</span>
    {:else}
      <span class="pct subtle">—</span>
    {/if}
  </div>
  <div class="track" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={known ? shown : undefined} aria-valuetext={known ? fmtLimit(pct, mode) : undefined} aria-label={title || windowLabel(win)}>
    <span class="fill {level}" style="width:{shown}%"></span>
  </div>
  <div class="foot">
    <span class="status {level}"><Icon name={icon} size={13} />{t(`limits.status.${level}`)}</span>
    {#if !compact}<AccuracyBadge kind={accuracy} compact />{/if}
    <span class="spacer"></span>
    {#if lstate === 'reset'}
      <span class="subtle">{t('limits.state.reset')}</span>
    {:else if lstate === 'stale'}
      <span class="subtle">{t('limits.state.stale')}</span>
    {:else if behind && observedMs}
      <span class="subtle" title={t('limits.state.behind.help')}>{t('limits.state.behind', { pct: fmtLimit(used ?? 0, mode), t: fmtDuration(now - observedMs) })}</span>
    {:else if resetsAt}
      <span class="subtle" title={fmtTime(resetsAt * 1000)}>{t('limits.resetsIn', { t: fmtDuration(resetsAt * 1000 - now) })}</span>
    {:else if observedMs}
      <span class="subtle">{t('limits.observed', { t: fmtDuration(now - observedMs) })}</span>
    {/if}
  </div>
  {#if known && forecast && (!compact || forecast.kind === 'fills')}
    {@const f = forecast}
    <p class="forecast" class:warn={f.kind === 'fills'} title={t('forecast.help')}>
      <Icon name={f.kind === 'fills' ? 'warning' : f.kind === 'insufficient' ? 'clock' : 'chart'} size={13} />
      <span>
        {#if f.kind === 'fills' && f.fills_at_ms}
          {compact ? t('forecast.fillsShort', { t: fmtWhen(f.fills_at_ms) }) : t('forecast.fills', { t: fmtWhen(f.fills_at_ms), d: fmtDuration((resetsAt ?? 0) * 1000 - f.fills_at_ms) })}
        {:else if f.kind === 'safe' && f.at_reset_pct !== null}
          {t('forecast.safe', { pct: fmtLimit(f.at_reset_pct, mode) })}
        {:else if f.kind === 'idle'}
          {t('forecast.idle')}
        {:else}
          {t('forecast.insufficient')}
        {/if}
        {#if !compact && f.rate_per_hour !== null && (f.kind === 'fills' || f.kind === 'safe')}
          <span class="subtle"> · {t('forecast.rate', { n: fmtDec(f.rate_per_hour, 1) })}</span>
        {/if}
      </span>
      {#if !compact}<AccuracyBadge kind="estimated" compact />{/if}
    </p>
  {/if}
  {#if behind && !compact}
    <p class="hint subtle">
      {sinceTokens > 0 ? t('limits.state.behind.since', { n: fmtCompact(sinceTokens) }) + ' ' : ''}{provider === 'anthropic' ? t('limits.state.behind.claudeHint') : t('limits.state.behind.hint')}
    </p>
  {/if}
  {#if source && !compact}
    <div class="src subtle">{t(`limits.source.${source}`)}</div>
  {/if}
</div>

<style>
  .meter {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .name {
    font-weight: 600;
  }
  .pct {
    font-size: 20px;
    font-weight: 650;
    letter-spacing: -0.02em;
  }
  .compact .pct {
    font-size: 15px;
  }
  .track {
    height: 8px;
    border-radius: 4px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: 4px;
    background: var(--accent);
    transition: width 600ms var(--ease);
  }
  .fill.warn {
    background: var(--warning);
  }
  .fill.high {
    background: var(--serious);
  }
  .fill.full {
    background: var(--critical);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    flex-wrap: wrap;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--ink-2);
    font-weight: 500;
  }
  .status.ok :global(svg) {
    color: var(--good-ink);
  }
  .status.full :global(svg),
  .status.high :global(svg) {
    color: var(--critical);
  }
  .status.warn :global(svg) {
    color: var(--serious);
  }
  .src {
    font-size: 11.5px;
  }
  .forecast {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 12px;
    color: var(--ink-2);
  }
  .forecast span {
    flex: 1;
  }
  .forecast.warn {
    color: var(--ink);
  }
  .forecast.warn :global(svg) {
    color: var(--serious);
  }
  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.45;
  }
  .spacer {
    flex: 1;
  }
</style>
