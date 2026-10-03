<script lang="ts">
  import { app, refresh, setFilter } from '../lib/store.svelte'
  import { api, type ProjectRow } from '../lib/api'
  import { fmtCompact, fmtMoney, fmtPct, t } from '../lib/i18n.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Icon from '../components/Icon.svelte'
  import Branches from '../components/Branches.svelte'

  let rows: ProjectRow[] = $state([])
  $effect(() => {
    void app.tick
    api.projectsForSettings().then((r) => (rows = r))
  })

  const byId = $derived(new Map((app.report?.by_project ?? []).map((g) => [g.key, g])))
  const total = $derived(Math.max(1e-9, app.report?.totals.cost_usd ?? 0))
  const hideAll = $derived(!!app.settings?.hide_project_names)
  const sorted = $derived(
    [...rows].sort((a, b) => (byId.get(String(b.id))?.totals.cost_usd ?? -1) - (byId.get(String(a.id))?.totals.cost_usd ?? -1)),
  )
  const noProject = $derived(byId.get('none'))

  async function toggle(p: ProjectRow, hidden: boolean) {
    await api.setProjectHidden(p.id, hidden)
    p.hidden = hidden
    refresh()
  }
  function focus(id: number) {
    setFilter({ ...app.filter, projects: [id] })
    app.view = 'overview'
  }
  const name = (p: ProjectRow) => (hideAll || p.hidden ? `${t('projects.hidden')} #${p.id}` : p.name)
</script>

<p class="subtle small help"><Icon name="lock" size={13} /> {t('projects.help')}</p>

<section class="card list-card">
  {#each sorted as p (p.id)}
    {@const g = byId.get(String(p.id))}
    <div class="list-row">
      <button class="name" onclick={() => focus(p.id)} title={t('projects.filter')} disabled={!g}>
        <Icon name="folder" size={16} />
        <span class="txt">{name(p)}</span>
        {#if !hideAll && !p.hidden}<span class="path subtle small">{p.path}</span>{/if}
      </button>
      {#if g}
        <span class="num">{fmtCompact(g.totals.total_tokens)}</span>
        <span class="num money">{fmtMoney(g.totals.cost_usd)}</span>
        <span class="num subtle share">{fmtPct((g.totals.cost_usd / total) * 100, 1)}</span>
      {:else}
        <span class="subtle small idle">—</span>
      {/if}
      <label class="hide">
        <span class="small muted">{t('projects.hide')}</span>
        <Toggle checked={p.hidden} label="{t('projects.hide')}: {name(p)}" onchange={(v) => toggle(p, v)} />
      </label>
    </div>
  {:else}
    <p class="muted">{t('common.empty')}</p>
  {/each}
  {#if noProject}
    <div class="list-row">
      <span class="name static"><Icon name="folder" size={16} /><span class="txt muted">{t('projects.noProject')}</span></span>
      <span class="num">{fmtCompact(noProject.totals.total_tokens)}</span>
      <span class="num money">{fmtMoney(noProject.totals.cost_usd)}</span>
      <span class="num subtle share">{fmtPct((noProject.totals.cost_usd / total) * 100, 1)}</span>
      <span class="hide"></span>
    </div>
  {/if}
</section>

<Branches />

<style>
  .help {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin-bottom: 12px;
    max-width: 760px;
  }
  .list-card {
    padding: 4px 20px;
  }
  .list-row {
    display: grid;
    grid-template-columns: 1fr 90px 90px 60px 150px;
    gap: 14px;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    border: 0;
    background: transparent;
    padding: 6px 8px;
    margin-left: -8px;
    border-radius: 8px;
    text-align: left;
  }
  button.name:hover:not(:disabled) {
    background: var(--surface-hover);
  }
  .name :global(svg) {
    color: var(--ink-3);
  }
  .txt {
    font-weight: 500;
    white-space: nowrap;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .idle {
    grid-column: span 3;
    text-align: right;
  }
  .hide {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
