<script lang="ts">
  import { app, latest, setFilter, setPeriod } from '../lib/store.svelte'
  import { localDate, t, toolLabel } from '../lib/i18n.svelte'
  import { api, type Period, type Tool } from '../lib/api'
  import Segmented from './Segmented.svelte'
  import Select from './Select.svelte'
  import Icon from './Icon.svelte'

  type Kind = Period['kind']
  const kinds: Kind[] = ['today', 'days7', 'month1', 'months3', 'months6', 'year1', 'all', 'custom']
  const today = new Date()
  let kind: Kind = $state(app.period.kind)
  let from = $state(app.period.kind === 'custom' ? app.period.from : localDate(new Date(today.getFullYear(), today.getMonth(), today.getDate() - 13)))
  let to = $state(app.period.kind === 'custom' ? app.period.to : localDate(today))
  let showFilters = $state(false)
  let models: string[] = $state([])
  let projects: { id: number; name: string; hidden: boolean }[] = $state([])

  // refetched on every data change too, so hiding a project never leaves its real name in the list
  $effect(() => {
    if (!showFilters) return
    void app.tick
    const stop = [latest(() => api.models(), (m) => (models = m)), latest(() => api.projects(), (p) => (projects = p))]
    return () => stop.forEach((f) => f())
  })

  const customError = $derived(!from || !to ? t('period.invalid.empty') : from > to ? t('period.invalid.order') : '')
  function pick(k: Kind) {
    if (k !== 'custom') setPeriod({ kind: k } as Period)
  }
  function applyCustom() {
    if (customError) return
    setPeriod({ kind: 'custom', from, to })
  }
  const activeFilters = $derived((app.filter.tools?.length ?? 0) + (app.filter.models?.length ?? 0) + (app.filter.projects?.length ?? 0))
  const tools: Tool[] = ['claude_code', 'codex']
</script>

<header class="bar">
  <h1>{t(`nav.${app.view}`)}</h1>
  <div class="spacer"></div>
  <Segmented label={t('period.label')} options={kinds.map((k) => ({ value: k, label: t(`period.${k}`) }))} bind:value={kind} onchange={pick} />
  <button class="btn" class:primary={activeFilters > 0} aria-expanded={showFilters} onclick={() => (showFilters = !showFilters)}>
    <Icon name="filter" size={15} />{t('common.filters')}{activeFilters ? ` · ${activeFilters}` : ''}
  </button>
</header>
{#if kind === 'custom'}
  <div class="sub">
    <label>{t('period.from')} <input class="field" type="date" bind:value={from} max={to} /></label>
    <label>{t('period.to')} <input class="field" type="date" bind:value={to} min={from} /></label>
    <button class="btn primary" onclick={applyCustom} disabled={!!customError}>{t('period.apply')}</button>
    {#if customError}<span class="err" role="alert">{customError}</span>{/if}
  </div>
{/if}
{#if showFilters}
  <div class="sub filters">
    <label>
      {t('common.tool')}
      <Select label={t('common.tool')} value={app.filter.tools?.[0] ?? ''} minWidth={140} options={[{ value: '', label: t('common.all') }, ...tools.map((tl) => ({ value: tl, label: toolLabel(tl) }))]} onchange={(v) => setFilter({ ...app.filter, tools: v ? [v as Tool] : [] })} />
    </label>
    <label>
      {t('common.model')}
      <Select label={t('common.model')} value={app.filter.models?.[0] ?? ''} minWidth={180} searchable options={[{ value: '', label: t('common.all') }, ...models.map((m) => ({ value: m, label: m }))]} onchange={(v) => setFilter({ ...app.filter, models: v ? [v] : [] })} />
    </label>
    <label>
      {t('common.project')}
      <Select
        label={t('common.project')}
        value={String(app.filter.projects?.[0] ?? '')}
        minWidth={180}
        searchable
        options={[{ value: '', label: t('common.all') }, ...projects.map((p) => ({ value: String(p.id), label: p.hidden ? `${t('projects.hidden')} #${p.id}` : p.name }))]}
        onchange={(v) => setFilter({ ...app.filter, projects: v ? [Number(v)] : [] })}
      />
    </label>
    {#if activeFilters}
      <button class="btn ghost" onclick={() => setFilter({})}>{t('common.clear')}</button>
    {/if}
  </div>
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 22px 28px 12px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 28px 12px;
    flex-wrap: wrap;
    font-size: 13px;
    color: var(--ink-2);
  }
  .sub label {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .err {
    color: var(--bad-ink);
  }
  @container main (max-width: 1040px) {
    .bar {
      padding: 18px 20px 10px;
    }
    .sub {
      padding: 0 20px 12px;
    }
  }

</style>
