<script lang="ts">
  import { fmtCompact, fmtPct, lower, t } from '../../lib/i18n.svelte'
  import type { Tokens } from '../../lib/api'
  let { tokens }: { tokens: Tokens } = $props()

  const parts = $derived([
    { key: 'input', label: t('metric.input'), value: tokens.input, color: 'var(--s1)' },
    { key: 'output', label: t('metric.output'), value: tokens.output, color: 'var(--s2)' },
    { key: 'cache_read', label: t('metric.cacheRead'), value: tokens.cache_read, color: 'var(--s3)' },
    { key: 'cache_write', label: t('metric.cacheWrite'), value: tokens.cache_write, color: 'var(--s4)' },
  ])
  const total = $derived(Math.max(1, parts.reduce((a, p) => a + p.value, 0)))
</script>

<div class="bar" role="img" aria-label={parts.map((p) => `${p.label} ${fmtPct((p.value / total) * 100, 1)}`).join(', ')}>
  {#each parts.filter((p) => p.value > 0) as p (p.key)}
    <span style="flex:{p.value};background:{p.color}" title="{p.label}: {fmtPct((p.value / total) * 100, 1)}"></span>
  {/each}
</div>
<table class="legend">
  <tbody>
    {#each parts as p (p.key)}
      <tr>
        <td><i class="swatch" style="background:{p.color}"></i></td>
        <td class="name">{p.label}</td>
        <td class="num">{fmtCompact(p.value)}</td>
        <td class="num subtle">{fmtPct((p.value / total) * 100, 1)}</td>
      </tr>
    {/each}
    {#if tokens.reasoning > 0}
      <tr class="note">
        <td></td>
        <td class="name" title={t('metric.reasoning.help')}>↳ {t('metric.reasoning')}</td>
        <td class="num">{fmtCompact(tokens.reasoning)}</td>
        <td class="num subtle">{fmtPct((tokens.reasoning / Math.max(1, tokens.output)) * 100, 0)} {lower(t('metric.output'))}</td>
      </tr>
    {/if}
  </tbody>
</table>

<style>
  .bar {
    display: flex;
    gap: 2px;
    height: 12px;
    border-radius: 4px;
    overflow: hidden;
    margin: 4px 0 14px;
  }
  .bar span {
    min-width: 3px;
  }
  .legend {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  td {
    padding: 4px 0;
  }
  td:first-child {
    width: 18px;
  }
  .name {
    color: var(--ink-2);
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    padding-left: 12px;
  }
  .note .name {
    color: var(--ink-3);
    font-size: 12px;
  }
</style>
