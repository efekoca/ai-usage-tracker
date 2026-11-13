<script lang="ts">
  import { app, latest } from '../lib/store.svelte'
  import { api, type StartingLoad } from '../lib/api'
  import { cacheClient } from '../lib/caches'
  import { fmtCompact, fmtDate, fmtDateTime, fmtInt, fmtMoney, t, toolLabel } from '../lib/i18n.svelte'

  const [W, H, PAD] = [132, 30, 4]

  const FIRST = 10

  let loads = $state<StartingLoad[]>([])
  let all = $state(false)
  // the conversation under the pointer: its row and its place in the line
  let hover = $state<{ row: string; i: number } | null>(null)
  $effect(() => {
    void app.tick
    return latest(
      () => api.startingLoads($state.snapshot(app.period), $state.snapshot(app.filter)),
      (d) => {
        loads = d
        // the lines may have changed under the pointer
        hover = null
      },
      () => (loads = []),
    )
  })

  const name = (l: StartingLoad) => (l.hidden ? `${t('projects.hidden')}${l.project_id === null ? '' : ` #${l.project_id}`}` : l.project || t('context.start.noProject'))
  const where = (l: StartingLoad) => [toolLabel(l.tool), cacheClient(l.client)].filter(Boolean).join(' · ')
  const delta = (l: StartingLoad) => (l.change ? l.change.after - l.change.before : 0)
  const signed = (n: number) => `${n > 0 ? '+' : '−'}${fmtCompact(Math.abs(n))}`

  /** The conversations' first requests as a line, left to right, with the point a change began at. */
  function line(l: StartingLoad) {
    const values = l.starts.map((s) => s.tokens)
    const [min, max] = [Math.min(...values), Math.max(...values)]
    // the height stands for at least a sixth of the largest, so a few hundred tokens' wobble stays a flat line
    const span = Math.max(max - min, max * 0.16, 1)
    const low = (min + max) / 2 - span / 2
    const x = (i: number) => (values.length < 2 ? W / 2 : PAD + (i * (W - 2 * PAD)) / (values.length - 1))
    const y = (v: number) => H - PAD - ((v - low) * (H - 2 * PAD)) / span
    const at = l.change ? l.starts.findIndex((s) => s.ts_ms === l.change?.at_ms) : -1
    const at_ = (i: number) => ({ x: x(i), y: y(values[i]) })
    return {
      points: values.map((_, i) => `${at_(i).x.toFixed(1)},${at_(i).y.toFixed(1)}`).join(' '),
      last: at_(values.length - 1),
      mark: at < 0 ? null : at_(at),
      at: at_,
    }
  }
  const rowKey = (l: StartingLoad) => `${l.project_id}:${l.tool}:${l.client}`
  /** The conversation nearest the pointer along the line. */
  function point(e: PointerEvent, l: StartingLoad) {
    const box = (e.currentTarget as Element).getBoundingClientRect()
    const px = box.width ? ((e.clientX - box.left) / box.width) * W : e.clientX
    const last = l.starts.length - 1
    const i = last < 1 ? 0 : Math.min(last, Math.max(0, Math.round(((px - PAD) / (W - 2 * PAD)) * last)))
    hover = { row: rowKey(l), i }
  }
</script>

{#if loads.length}
  <section class="card table-card">
    <h2 class="pad">{t('context.start')}</h2>
    <p class="subtle small help">{t('context.start.help')}</p>
    <div class="scroll">
      <table>
        <thead>
          <tr>
            <th scope="col">{t('common.project')}</th>
            <th scope="col" class="num">{t('context.start.conversations')}</th>
            <th scope="col" class="num">{t('context.start.typical')}</th>
            <th scope="col" class="num">{t('context.start.smallest')}</th>
            <th scope="col" class="num">{t('context.start.cost')}</th>
            <th scope="col">{t('context.start.course')}</th>
            <th scope="col">{t('context.start.change')}</th>
          </tr>
        </thead>
        <tbody>
          {#each all ? loads : loads.slice(0, FIRST) as l (rowKey(l))}
            {@const s = line(l)}
            {@const over = hover?.row === rowKey(l) && hover.i < l.starts.length ? hover.i : -1}
            <tr>
              <th scope="row">
                <span class="project">{name(l)}</span>
                <span class="subtle small">{where(l)}</span>
              </th>
              <td class="num">{fmtInt(l.conversations)}</td>
              <td class="num"><b>{fmtCompact(l.typical)}</b></td>
              <td class="num">{fmtCompact(l.smallest)}</td>
              <td class="num">{l.cost_usd === null ? '—' : fmtMoney(l.cost_usd)}</td>
              <td>
                <span class="chart">
                  <svg
                    class="spark"
                    width={W}
                    height={H}
                    viewBox="0 0 {W} {H}"
                    role="img"
                    aria-label={t('context.start.courseOf', { n: l.starts.length, from: fmtCompact(l.starts[0].tokens), to: fmtCompact(l.starts[l.starts.length - 1].tokens) })}
                    onpointermove={(e) => point(e, l)}
                    onpointerdown={(e) => point(e, l)}
                    onpointerleave={() => (hover = null)}
                  >
                    {#if s.mark}<line class="since" x1={s.mark.x} y1="1" x2={s.mark.x} y2={H - 1} />{/if}
                    {#if l.starts.length > 1}<polyline points={s.points} />{/if}
                    <circle class="now" cx={s.last.x} cy={s.last.y} r="2.6" />
                    {#if over >= 0}<circle class="at" cx={s.at(over).x} cy={s.at(over).y} r="3.4" />{/if}
                  </svg>
                  {#if over >= 0}
                    <span class="tip">
                      <b class="num">{fmtCompact(l.starts[over].tokens)}</b>
                      <span>{fmtDateTime(l.starts[over].ts_ms)}</span>
                    </span>
                  {/if}
                </span>
              </td>
              <td>
                {#if l.change}
                  <span class="step" class:up={delta(l) > 0} title={t('context.start.since', { date: fmtDate(l.change.at_ms, 'medium'), before: fmtCompact(l.change.before), after: fmtCompact(l.change.after) })}>
                    <b class="num">{signed(delta(l))}</b>
                    <span class="subtle small">{fmtDate(l.change.at_ms, 'short')}</span>
                  </span>
                {:else}
                  <span class="subtle">—</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if !all && loads.length > FIRST}
      <button class="btn ghost small more" onclick={() => (all = true)}>{t('tools.showAll', { n: loads.length })}</button>
    {/if}
    <p class="subtle small pad note">{t('context.start.note')}</p>
  </section>
{/if}

<style>
  .table-card {
    padding: 6px 0;
    margin-top: 16px;
  }
  h2 {
    margin-bottom: 0;
  }
  .help {
    margin: 0;
    padding: 0 20px 10px;
    max-width: 760px;
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
    vertical-align: middle;
  }
  thead th {
    font-weight: 500;
    color: var(--ink-2);
    font-size: 12px;
    vertical-align: bottom;
  }
  .table-card th:first-child,
  .table-card td:first-child {
    padding-left: 20px;
  }
  .table-card th:last-child,
  .table-card td:last-child {
    padding-right: 20px;
  }
  /* wide enough that a short name and where it runs stay on a line each when the table is narrow */
  tbody th {
    min-width: 170px;
    font-weight: 500;
  }
  tbody th span {
    display: block;
  }
  tbody th .small {
    white-space: nowrap;
  }
  .project {
    overflow-wrap: anywhere;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .chart {
    position: relative;
    display: inline-block;
  }
  .spark {
    display: block;
    cursor: crosshair;
    touch-action: none;
  }
  .at {
    fill: var(--surface);
    stroke: var(--s1);
    stroke-width: 2;
  }
  .tip {
    /* beside the line, where the table's scrolling box does not cut it off */
    position: absolute;
    right: calc(100% + 8px);
    top: 50%;
    z-index: 2;
    transform: translateY(-50%);
    display: grid;
    justify-items: end;
    gap: 1px;
    padding: 6px 9px;
    border-radius: 8px;
    background: var(--surface);
    border: 0.5px solid var(--hairline-strong);
    box-shadow: var(--shadow-pop);
    font-size: 12px;
    white-space: nowrap;
    pointer-events: none;
  }
  .tip span {
    color: var(--ink-3);
  }
  polyline {
    fill: none;
    stroke: var(--s1);
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .now {
    fill: var(--s1);
  }
  .since {
    stroke: var(--ink-3);
    stroke-width: 1;
    stroke-dasharray: 2 3;
  }
  .step {
    display: grid;
    gap: 1px;
    justify-items: start;
    white-space: nowrap;
  }
  .step b {
    font-weight: 600;
    color: var(--good-ink);
  }
  .step.up b {
    color: var(--bad-ink);
  }
  .more {
    margin: 8px 14px 0;
  }
  .pad {
    padding: 12px 20px 6px;
  }
  .note {
    padding-bottom: 14px;
  }
</style>
