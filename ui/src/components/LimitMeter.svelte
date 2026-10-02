<script lang="ts">
  // A plan-limit meter. Severity uses the reserved status colors and always ships with an
  // icon + label; stale/reset readings are shown as unknown instead of a misleading bar.
  import Icon from './Icon.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'
  import { fmtDuration, fmtPct, fmtTime, t, windowLabel } from '../lib/i18n.svelte'
  import type { Accuracy, LimitState } from '../lib/api'

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
  } = $props()

  let now = $state(Date.now())
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 30_000)
    return () => clearInterval(id)
  })

  // 'behind': the provider was used after this reading, so the real value is at least this
  const known = $derived((lstate === 'fresh' || lstate === 'behind') && used !== null)
  const atLeast = $derived(lstate === 'behind')
  const pct = $derived(known ? Math.max(0, Math.min(100, used as number)) : 0)
  const level = $derived(!known ? 'unknown' : pct >= 100 ? 'full' : pct >= 90 ? 'high' : pct >= 70 ? 'warn' : 'ok')
  const icon = $derived(level === 'ok' ? 'check' : level === 'unknown' ? 'clock' : 'warning')
</script>

<div class="meter" class:compact>
  <div class="head">
    <span class="name">{title || windowLabel(win)}</span>
    <span class="spacer"></span>
    {#if known}
      <span class="pct num">{atLeast ? '≥ ' : ''}{fmtPct(pct)}</span>
    {:else}
      <span class="pct subtle">—</span>
    {/if}
  </div>
  <div class="track" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={known ? pct : undefined} aria-label={title || windowLabel(win)}>
    <span class="fill {level}" style="width:{pct}%"></span>
  </div>
  <div class="foot">
    <span class="status {level}"><Icon name={icon} size={13} />{t(`limits.status.${level}`)}</span>
    {#if !compact}<AccuracyBadge kind={accuracy} compact />{/if}
    <span class="spacer"></span>
    {#if lstate === 'reset'}
      <span class="subtle">{t('limits.state.reset')}</span>
    {:else if lstate === 'stale'}
      <span class="subtle">{t('limits.state.stale')}</span>
    {:else if atLeast && observedMs}
      <span class="subtle" title={t('limits.state.behind.help')}>{t('limits.state.behind', { t: fmtDuration(now - observedMs) })}</span>
    {:else if resetsAt}
      <span class="subtle" title={fmtTime(resetsAt * 1000)}>{t('limits.resetsIn', { t: fmtDuration(resetsAt * 1000 - now) })}</span>
    {:else if observedMs}
      <span class="subtle">{t('limits.observed', { t: fmtDuration(now - observedMs) })}</span>
    {/if}
  </div>
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
  .spacer {
    flex: 1;
  }
</style>
