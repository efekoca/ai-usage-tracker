<script lang="ts">
  import { api, type SessionRow, type SessionTimeline, type TimelineRequest } from '../lib/api'
  import { app } from '../lib/store.svelte'
  import { agentLabel, fmtClock, fmtCompact, fmtDuration, fmtInt, fmtMoney, fmtPct, t } from '../lib/i18n.svelte'

  let { session }: { session: SessionRow } = $props()

  let data = $state<SessionTimeline | null>(null)
  let failed = $state('')
  $effect(() => {
    let live = true
    // the same filter as the session list, so the chart shows the requests the row counts
    api.sessionTimeline(session.tool, session.session_id, session.started_ms, session.ended_ms, $state.snapshot(app.filter)).then(
      (v) => {
        if (!live) return
        data = v
        failed = ''
      },
      (e) => live && (failed = String(e)),
    )
    return () => (live = false)
  })

  /** A pause this long lets Claude's cache lapse. */
  const PAUSE_MS = 5 * 60_000
  /** Room for the y labels, the notes above the columns, the columns, the time labels and the tool strip. */
  const L = 44
  const TOP = 34
  const PLOT = 192
  const BASE = TOP + PLOT
  const H = BASE + 58
  const NOTE_GAP = 150

  let width = $state(0)
  let hover = $state<number | null>(null)
  let tipH = $state(0)

  const reqs = $derived(data?.requests ?? [])
  const calls = $derived(data?.calls ?? [])
  const n = $derived(reqs.length)

  const scale = $derived.by(() => {
    const top = Math.max(1, ...reqs.map((r) => r.context))
    const raw = top / 3
    const mag = 10 ** Math.floor(Math.log10(raw))
    const step = Math.max(1, [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? raw)
    return { step, max: step * 3 }
  })
  const hgt = (v: number) => Math.round((PLOT * v) / scale.max)
  /** A subagent may have no name, but always has its own thread. */
  const sub = (r: TimelineRequest) => !!r.agent || !!r.thread
  const agentOf = (r: TimelineRequest) => r.agent ?? 'subagent'
  // the unnamed one's label already says it is a subagent
  const agentNote = (a: string) => (a === 'subagent' ? agentLabel(a) : t('timeline.ev.agent', { a: agentLabel(a) }))

  const family = getComputedStyle(document.body).fontFamily
  let pen: CanvasRenderingContext2D | null = null
  /** A label's width in the chart's 11 px type. */
  function textW(s: string, weight: number): number {
    pen ??= document.createElement('canvas').getContext('2d')
    if (!pen) return s.length * 6.5
    pen.font = `${weight} 11px ${family}`
    return pen.measureText(s).width
  }

  const geo = $derived.by(() => {
    const pauses = new Set(reqs.map((r, i) => (i > 0 && r.ts_ms - reqs[i - 1].ts_ms >= PAUSE_MS ? i : -1)).filter((i) => i > 0))
    const plotW = Math.max(0, width - L)
    const gap = pauses.size ? Math.min(36, (plotW * 0.2) / pauses.size) : 0
    const step = n ? (plotW - gap * pauses.size) / n : 0
    const pad = step >= 8 ? 4 : step >= 4 ? 2 : step >= 2 ? 1 : 0
    const bw = Math.max(1, Math.floor(step - pad))
    const xs: number[] = []
    let shift = 0
    for (let i = 0; i < n; i++) {
      if (pauses.has(i)) shift += gap
      xs.push(Math.round(L + shift + step * i + pad / 2))
    }
    return { pauses: [...pauses], gap, bw, xs }
  })

  /** A column with only its top corners rounded. */
  function column(x: number, top: number, bottom: number): string {
    const w = geo.bw
    const r = Math.min(2.5, (bottom - top) / 2, w / 2)
    return `M${x} ${bottom}V${top + r}Q${x} ${top} ${x + r} ${top}H${x + w - r}Q${x + w} ${top} ${x + w} ${top + r}V${bottom}Z`
  }

  const bars = $derived(
    reqs.map((r, i) => {
      const x = geo.xs[i]
      const top = BASE - Math.max(1, hgt(r.context))
      if (r.rewritten <= 0) return { body: column(x, top, BASE), cap: '', agent: sub(r) }
      const split = Math.min(BASE - 2, top + Math.max(2, hgt(r.rewritten)))
      return { body: `M${x} ${BASE}V${split + 1}H${x + geo.bw}V${BASE}Z`, cap: column(x, top, split), agent: sub(r) }
    }),
  )

  const grid = $derived([1, 2, 3].map((k) => ({ y: BASE - hgt(scale.step * k) + 0.5, label: fmtCompact(scale.step * k) })))

  /** A pause's length sits in the time row under its gap, which is too narrow to hold it. */
  const breaks = $derived.by(() => {
    let end = -Infinity
    return geo.pauses.map((i) => {
      const from = geo.xs[i - 1] + geo.bw
      const cx = (from + geo.xs[i]) / 2
      let label = geo.gap >= 30 ? fmtDuration(reqs[i].ts_ms - reqs[i - 1].ts_ms) : ''
      const half = label ? textW(label, 400) / 2 : 0
      const x = Math.min(Math.max(cx, L + half), width - half)
      if (label && x - half < end + 8) label = ''
      if (label) end = x + half
      return { x: from + 2, w: Math.max(0, geo.xs[i] - from - 4), cx: x, half, label }
    })
  })

  const ticks = $derived.by(() => {
    if (!n) return []
    const taken = breaks.filter((b) => b.label).map((b) => [b.cx - b.half - 8, b.cx + b.half + 8])
    const out: { x: number; label: string }[] = []
    for (const i of [...new Set([0, 1, 2, 3, 4, 5].map((k) => Math.round((k * (n - 1)) / 5)))]) {
      const x = geo.xs[i] + geo.bw / 2
      if (out.length && x - out[out.length - 1].x < 64) continue
      const label = fmtClock(reqs[i].ts_ms)
      const half = textW(label, 400) / 2
      if (taken.some(([a, b]) => x + half > a && x - half < b)) continue
      out.push({ x, label })
    }
    return out
  })

  const notes = $derived.by(() => {
    const out: { x: number; top: number; text: string; cls: string; anchor: string }[] = []
    reqs.forEach((r, i) => {
      const text = r.rewritten > 0 ? t('timeline.note.rewrite') : r.compacted ? t('timeline.note.compact') : ''
      if (!text) return
      const x = geo.xs[i] + geo.bw / 2
      if (out.length && x - out[out.length - 1].x < NOTE_GAP) return
      const anchor = x < L + 80 ? 'start' : x > width - 80 ? 'end' : 'middle'
      out.push({ x, top: BASE - Math.max(1, hgt(r.context)), text, cls: r.rewritten > 0 ? 'rewrite' : 'compact', anchor })
    })
    return out
  })

  /** Each subagent run: the requests of its thread, which can interleave with others; without a thread, one agent's requests in a row. */
  const runs = $derived.by(() => {
    const out: { agent: string; thread: string | null; idx: number[] }[] = []
    const byThread = new Map<string, (typeof out)[number]>()
    reqs.forEach((r, i) => {
      if (!sub(r)) return
      const known = r.thread ? byThread.get(r.thread) : undefined
      if (known) return known.idx.push(i)
      const last = out[out.length - 1]
      if (!r.thread && last && !last.thread && last.agent === agentOf(r) && last.idx[last.idx.length - 1] === i - 1) return last.idx.push(i)
      const run = { agent: agentOf(r), thread: r.thread, idx: [i] }
      if (r.thread) byThread.set(r.thread, run)
      out.push(run)
    })
    return out
  })
  /** Runs that overlap share one bracket: subagents working at once interleave their requests. */
  const groups = $derived.by(() => {
    const spans = runs
      .map((run) => ({ a: geo.xs[run.idx[0]], b: geo.xs[run.idx[run.idx.length - 1]] + geo.bw, runs: 1, n: run.idx.length }))
      .sort((p, q) => p.a - q.a)
    const out: typeof spans = []
    for (const s of spans) {
      const g = out[out.length - 1]
      if (g && s.a <= g.b + 4) {
        g.b = Math.max(g.b, s.b)
        g.runs++
        g.n += s.n
      } else out.push({ ...s })
    }
    return out
  })

  /** Every label sits above the columns and the labels placed before it, so none overlap; the chart grows upward when they need the room. */
  const layout = $derived.by(() => {
    const w = Math.ceil(width) + 2
    const sky = new Array<number>(w).fill(BASE)
    const cols = (x0: number, x1: number) => [Math.max(0, Math.floor(x0)), Math.min(w - 1, Math.ceil(x1))]
    const floor = (x0: number, x1: number) => {
      const [p, q] = cols(x0, x1)
      let m = BASE
      for (let x = p; x <= q; x++) m = Math.min(m, sky[x])
      return m
    }
    const raise = (x0: number, x1: number, y: number) => {
      const [p, q] = cols(x0, x1)
      for (let x = p; x <= q; x++) sky[x] = Math.min(sky[x], y)
    }
    const CAP = 11
    reqs.forEach((r, i) => raise(geo.xs[i], geo.xs[i] + geo.bw, BASE - Math.max(1, hgt(r.context))))
    // a note points at one column, so it goes nearest to it
    const placedNotes = notes.map((note) => {
      const lw = textW(note.text, 500)
      const x0 = note.anchor === 'start' ? note.x : note.anchor === 'end' ? note.x - lw : note.x - lw / 2
      const y = Math.min(note.top - 20, floor(x0 - 3, x0 + lw + 3) - 6)
      raise(x0 - 3, x0 + lw + 3, y - CAP)
      raise(note.x - 1, note.x + 1, y)
      return { ...note, y }
    })
    const placedBrackets = groups.map((g) => {
      const label = g.runs > 1 ? t('timeline.agentRuns', { k: g.runs, n: g.n }) : t('timeline.agentRun', { n: g.n })
      const lw = textW(label, 500)
      const cx = (g.a + g.b) / 2
      const y = floor(g.a, g.b) - 10
      raise(g.a, g.b, y)
      const shown = g.b - g.a >= lw + 8
      const ly = shown ? Math.min(y - 6, floor(cx - lw / 2 - 3, cx + lw / 2 + 3) - 4) : y
      if (shown) raise(cx - lw / 2 - 3, cx + lw / 2 + 3, ly - CAP)
      return { d: `M${g.a} ${y + 5}V${y}H${g.b}V${y + 5}`, x: cx, y: ly, label: shown ? label : '' }
    })
    return { notes: placedNotes, brackets: placedBrackets, lift: Math.max(0, Math.ceil(4 - Math.min(...sky))) }
  })

  /** The request each call belongs to: the last one sent before it. */
  function slotOf(ms: number): number {
    let lo = 0
    let hi = n - 1
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2)
      if (reqs[mid].ts_ms <= ms) lo = mid
      else hi = mid - 1
    }
    return lo
  }
  const strip = $derived(calls.map((c) => ({ x: geo.xs[slotOf(c.ts_ms)] + geo.bw / 2, failed: c.failed === true })))
  const failedCalls = $derived(calls.filter((c) => c.failed === true))

  const prompt = $derived(reqs.reduce((a, r) => a + r.cache_read + r.cache_write + fresh(r), 0))
  const sumOf = (f: (r: TimelineRequest) => number) => reqs.reduce((a, r) => a + f(r), 0)
  function fresh(r: TimelineRequest): number {
    return Math.max(0, r.context - r.cache_read - r.cache_write)
  }
  const costOf = (rs: TimelineRequest[]) => (rs.some((r) => r.cost_usd !== null) ? fmtMoney(rs.reduce((a, r) => a + (r.cost_usd ?? 0), 0)) : '—')
  const share = (v: number) => t('timeline.unit.share', { p: fmtPct(prompt ? (v / prompt) * 100 : 0) })

  const totals = $derived.by(() => {
    const agents = reqs.filter(sub)
    const read = sumOf((r) => r.cache_read)
    const write = sumOf((r) => r.cache_write)
    const out = [
      { cls: 'read', label: t('metric.cacheRead'), value: fmtCompact(read), unit: share(read), help: t('timeline.help.read') },
      { cls: 'write', label: t('metric.cacheWrite'), value: fmtCompact(write), unit: share(write), help: t('timeline.help.write') },
      { cls: 'fresh', label: t('timeline.fresh'), value: fmtCompact(sumOf(fresh)), unit: t('timeline.unit.tokens'), help: t('timeline.help.fresh') },
    ]
    out.push(
      agents.length
        ? { cls: 'agent', label: t('timeline.legend.agent'), value: fmtInt(agents.length), unit: t('timeline.unit.requests', { n: agents.length, cost: costOf(agents) }), help: t('timeline.help.agent') }
        : { cls: 'output', label: t('metric.output'), value: fmtCompact(sumOf((r) => r.output)), unit: t('timeline.unit.tokens'), help: t('timeline.help.output') },
    )
    return out
  })

  const events = $derived.by(() => {
    const out: { ms: number; cls: string; title: string; detail: string; value: string }[] = []
    reqs.forEach((r, i) => {
      if (r.rewritten > 0) {
        out.push({
          ms: r.ts_ms,
          cls: 'rewrite',
          title: t('timeline.ev.rewrite'),
          detail: t('timeline.ev.rewrite.detail', { d: fmtDuration(r.pause_ms ?? 0), n: fmtCompact(r.rewritten) }),
          value: r.rewrite_extra_usd === null ? '' : `+${fmtMoney(r.rewrite_extra_usd)}`,
        })
      }
      if (r.compacted) {
        // the request before can be one a filter left out, so the size comes with the mark
        const before = r.compacted_from
        out.push({ ms: r.ts_ms, cls: 'compact', title: t('timeline.ev.compact'), detail: before ? `${fmtCompact(before)} → ${fmtCompact(r.context)}` : '', value: before ? `−${fmtPct((1 - r.context / before) * 100)}` : '' })
      }
    })
    for (const run of runs) {
      const rs = run.idx.map((i) => reqs[i])
      out.push({
        ms: rs[0].ts_ms,
        cls: 'agent',
        title: agentNote(run.agent),
        detail: t('timeline.ev.agent.detail', { n: rs.length, max: fmtCompact(Math.max(...rs.map((r) => r.context))) }),
        value: costOf(rs),
      })
    }
    out.sort((a, b) => a.ms - b.ms)
    if (failedCalls.length) {
      const byName = new Map<string, number>()
      for (const c of failedCalls) byName.set(c.name, (byName.get(c.name) ?? 0) + 1)
      const list = [...byName].sort((a, b) => b[1] - a[1]).map(([name, k]) => `${name} ${k}`).join(', ')
      out.push({ ms: 0, cls: 'failed', title: t('timeline.ev.failed'), detail: t('timeline.ev.failed.detail', { f: fmtInt(failedCalls.length), n: fmtInt(calls.length), list }), value: fmtInt(failedCalls.length) })
    }
    return out
  })
  let allEvents = $state(false)

  const hr = $derived(hover === null ? null : reqs[hover])
  const tip = $derived.by(() => {
    if (hover === null || !hr) return null
    const cx = geo.xs[hover] + geo.bw / 2
    const top = BASE - Math.max(1, hgt(hr.context)) + layout.lift
    const above = top - 14 - tipH
    // above the column when it fits, else beside it
    const left = above >= 0 ? Math.min(Math.max(cx - 106, 0), width - 212) : cx + 16 + 212 <= width ? cx + 16 : cx - 16 - 212
    const note = hr.rewritten > 0 ? t('timeline.tip.rewrite', { d: fmtDuration(hr.pause_ms ?? 0) }) : sub(hr) ? agentNote(agentOf(hr)) : hr.compacted ? t('timeline.tip.compact') : ''
    return { left, top: above >= 0 ? above : 4, note, cls: hr.rewritten > 0 ? 'rewrite' : sub(hr) ? 'agent' : '' }
  })

  function move(e: PointerEvent) {
    const box = (e.currentTarget as SVGElement).getBoundingClientRect()
    const px = L + e.clientX - box.left
    let lo = 0
    let hi = n - 1
    while (lo < hi) {
      const mid = Math.floor((lo + hi) / 2)
      if (geo.xs[mid] + geo.bw / 2 < px) lo = mid + 1
      else hi = mid
    }
    const i = lo > 0 && Math.abs(geo.xs[lo - 1] + geo.bw / 2 - px) < Math.abs(geo.xs[lo] + geo.bw / 2 - px) ? lo - 1 : lo
    if (i !== hover) hover = i
  }

  // the arrow keys move through the requests, and a screen reader hears each one
  const STEPS: Record<string, number> = { ArrowRight: 1, ArrowUp: 1, ArrowLeft: -1, ArrowDown: -1, PageUp: 10, PageDown: -10 }
  function key(e: KeyboardEvent) {
    if (!n) return
    const at = hover ?? n - 1
    let i: number
    if (e.key in STEPS) i = Math.min(n - 1, Math.max(0, at + STEPS[e.key]))
    else if (e.key === 'Home') i = 0
    else if (e.key === 'End') i = n - 1
    else return
    e.preventDefault()
    hover = i
  }
  const spoken = $derived.by(() => {
    const r = reqs[hover ?? n - 1]
    if (!r) return ''
    const note = r.rewritten > 0 ? t('timeline.note.rewrite') : r.compacted ? t('timeline.note.compact') : sub(r) ? agentNote(agentOf(r)) : ''
    return [fmtClock(r.ts_ms), `${t('timeline.context')} ${fmtCompact(r.context)}`, `${t('metric.cost')} ${r.cost_usd === null ? '—' : fmtMoney(r.cost_usd)}`, note].filter(Boolean).join(', ')
  })

  const maxContext = $derived(Math.max(0, ...reqs.map((r) => r.context)))
  const hit = $derived(prompt ? (sumOf((r) => r.cache_read) / prompt) * 100 : 0)
</script>

<div class="timeline">
  <div class="head">
    <h3>{t('timeline.title')}</h3>
    {#if n}
      <span class="key"><i class="read"></i>{t('timeline.legend.context')}</span>
      {#if reqs.some((r) => r.rewritten > 0)}<span class="key"><i class="write"></i>{t('timeline.legend.rewrite')}</span>{/if}
      {#if runs.length}<span class="key"><i class="agent"></i>{t('timeline.legend.agent')}</span>{/if}
      <span class="stats">
        <span>{t('timeline.stat.max')} <b>{fmtCompact(maxContext)}</b></span>
        <span>{t('timeline.stat.hit')} <b>{fmtPct(hit, 1)}</b></span>
      </span>
    {/if}
  </div>
  {#if failed}
    <p class="muted small">{t('common.error', { e: failed })}</p>
  {:else if !data}
    <p class="muted small">{t('common.loading')}</p>
  {:else if !n}
    <p class="muted small">{t('timeline.empty')}</p>
  {:else}
    <div
      class="chart"
      bind:clientWidth={width}
      role="slider"
      tabindex="0"
      aria-label={t('timeline.requests')}
      aria-valuemin={1}
      aria-valuemax={n}
      aria-valuenow={(hover ?? n - 1) + 1}
      aria-valuetext={spoken}
      onkeydown={key}
      onfocus={() => hover === null && (hover = n - 1)}
      onblur={() => (hover = null)}
    >
      {#if width}
        <svg {width} height={(calls.length ? H : BASE + 26) + layout.lift} role="img" aria-label={t('timeline.aria', { n: fmtInt(n), max: fmtCompact(maxContext) })}>
          <g transform="translate(0 {layout.lift})">
          {#each grid as g, k (k)}
            <line class="grid" x1={L} x2={width} y1={g.y} y2={g.y} />
            <text class="axis" x="0" y={g.y + 3.5}>{g.label}</text>
          {/each}
          {#each layout.brackets as b, i (i)}
            <path class="bracket" d={b.d} />
            {#if b.label}<text class="bracket-label" x={b.x} y={b.y} text-anchor="middle">{b.label}</text>{/if}
          {/each}
          {#each bars as b, i (i)}
            <g class:dim={hover !== null && hover !== i}>
              <path class={b.agent ? 'agent' : 'read'} d={b.body} />
              {#if b.cap}<path class="write" d={b.cap} />{/if}
            </g>
          {/each}
          {#each layout.notes as note (note.x)}
            <line class="note-line {note.cls}" x1={note.x} x2={note.x} y1={note.y + 6} y2={note.top - 4} />
            <text class="note {note.cls}" x={note.x} y={note.y} text-anchor={note.anchor}>{note.text}</text>
          {/each}
          <line class="baseline" x1={L} x2={width} y1={BASE + 0.5} y2={BASE + 0.5} />
          {#each breaks as b, k (k)}
            <rect class="break" x={b.x} y={BASE - 1} width={b.w} height="3" />
            {#if b.label}<text class="axis" x={b.cx} y={BASE + 20} text-anchor="middle">{b.label}</text>{/if}
          {/each}
          {#each ticks as tk (tk.x)}
            <text class="axis" x={tk.x} y={BASE + 20} text-anchor="middle">{tk.label}</text>
          {/each}
          {#if calls.length}
            <text class="axis" x="0" y={BASE + 47}>{t('timeline.tools')}</text>
            {#each strip as c, i (i)}
              <rect class="call" class:failed={c.failed} x={c.x - (c.failed ? 1.5 : 1)} y={BASE + (c.failed ? 36 : 39)} width={c.failed ? 3 : 2} height={c.failed ? 14 : 8} rx="1" />
            {/each}
          {/if}
          <rect class="hit" x={L} y="0" width={Math.max(0, width - L)} height={BASE} role="presentation" onpointermove={move} onpointerleave={() => (hover = null)} />
          </g>
        </svg>
        {#if tip && hr}
          <div class="tip" style:left="{tip.left}px" style:top="{tip.top}px" bind:clientHeight={tipH}>
            <div class="tip-head"><span>{fmtClock(hr.ts_ms)}</span><span>{sub(hr) ? t('timeline.tip.agent') : t('timeline.tip.main')}</span></div>
            <div class="tip-context"><span>{t('timeline.context')}</span><b>{fmtCompact(hr.context)}</b></div>
            <div class="tip-row"><i class="read"></i><span>{t('metric.cacheRead')}</span><b>{fmtCompact(hr.cache_read)}</b></div>
            <div class="tip-row"><i class="write"></i><span>{t('metric.cacheWrite')}</span><b>{fmtCompact(hr.cache_write)}</b></div>
            <div class="tip-row"><i class="fresh"></i><span>{t('timeline.fresh')}</span><b>{fmtCompact(fresh(hr))}</b></div>
            <div class="tip-cost"><span>{t('metric.cost')}</span><b>{hr.cost_usd === null ? '—' : fmtMoney(hr.cost_usd)}</b></div>
            {#if tip.note}<div class="tip-note {tip.cls}">{tip.note}</div>{/if}
          </div>
        {/if}
      {/if}
    </div>
    {#if data.calls_hidden}<p class="subtle small calls-note">{t('timeline.callsHidden')}</p>{/if}

    <div class="totals">
      {#each totals as s (s.cls)}
        <div class="total">
          <span class="key"><i class={s.cls}></i>{s.label}</span>
          <span class="amount">{s.value} <small>{s.unit}</small></span>
          <span class="help">{s.help}</span>
        </div>
      {/each}
    </div>

    {#if events.length}
      <ul class="events">
        {#each allEvents ? events : events.slice(0, 6) as e, i (i)}
          <li>
            <span class="time">{e.ms ? fmtClock(e.ms) : ''}</span>
            <i class={e.cls}></i>
            <span class="title">{e.title}</span>
            <span class="detail">{e.detail}</span>
            <span class="value {e.cls}">{e.value}</span>
          </li>
        {/each}
      </ul>
      {#if events.length > 6 && !allEvents}
        <button class="btn ghost small" onclick={() => (allEvents = true)}>{t('timeline.more', { n: events.length - 6 })}</button>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .timeline {
    /* the warning and subagent colors, deepened for text on a light surface */
    --warn-text: color-mix(in srgb, var(--warning) 50%, var(--ink));
    --agent-text: color-mix(in srgb, var(--s3) 64%, var(--ink));
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    gap: 16px;
    margin-top: 8px;
    padding-top: 16px;
    box-shadow: inset 0 1px 0 var(--hairline);
    font-variant-numeric: tabular-nums;
  }
  .head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 20px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .key {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--ink-2);
  }
  .stats {
    display: inline-flex;
    gap: 20px;
    margin-left: auto;
    font-size: 12px;
    color: var(--ink-3);
    white-space: nowrap;
  }
  .stats b {
    font-weight: 500;
    color: var(--ink);
  }
  i {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }
  i.read,
  path.read {
    background: var(--s1);
    fill: var(--s1);
  }
  i.write,
  path.write {
    background: var(--warning);
    fill: var(--warning);
  }
  i.fresh,
  i.output {
    background: var(--ink-3);
    opacity: 0.6;
  }
  i.agent,
  path.agent {
    background: var(--s3);
    fill: var(--s3);
  }
  i.compact {
    background: var(--ink-3);
  }
  i.failed {
    background: var(--critical);
  }
  .chart {
    position: relative;
  }
  svg {
    display: block;
    overflow: visible;
  }
  .grid {
    stroke: var(--grid);
    stroke-dasharray: 2 4;
    shape-rendering: crispEdges;
  }
  .axis {
    fill: var(--ink-3);
    font-size: 11px;
  }
  .baseline {
    stroke: var(--hairline-strong);
    shape-rendering: crispEdges;
  }
  .break {
    fill: var(--surface);
  }
  .dim {
    opacity: 0.32;
  }
  .bracket {
    fill: none;
    stroke: var(--s3);
    stroke-opacity: 0.7;
    shape-rendering: crispEdges;
  }
  .bracket-label {
    fill: var(--agent-text);
    font-size: 11px;
    font-weight: 500;
  }
  .note {
    font-size: 11px;
    font-weight: 500;
  }
  .note.rewrite {
    fill: var(--warn-text);
  }
  .note.compact {
    fill: var(--ink-2);
  }
  .note-line {
    shape-rendering: crispEdges;
  }
  .note-line.rewrite {
    stroke: var(--warning);
  }
  .note-line.compact {
    stroke: var(--ink-3);
  }
  .call {
    fill: var(--ink-3);
    opacity: 0.55;
  }
  .call.failed {
    fill: var(--critical);
    opacity: 1;
  }
  .hit {
    fill: transparent;
  }
  .tip {
    position: absolute;
    width: 212px;
    box-sizing: border-box;
    padding: 10px 12px 11px;
    border-radius: 10px;
    background: var(--surface-2);
    box-shadow: 0 0 0 1px var(--hairline), var(--shadow-pop);
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: 5px;
    pointer-events: none;
    z-index: 5;
  }
  .tip-head,
  .tip-context,
  .tip-row,
  .tip-cost {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .tip-head {
    justify-content: space-between;
    color: var(--ink-3);
  }
  .tip-context {
    justify-content: space-between;
    color: var(--ink-2);
    margin-bottom: 2px;
  }
  .tip-context b {
    color: var(--ink);
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  .tip-row span,
  .tip-cost span {
    color: var(--ink-2);
  }
  .tip-row b,
  .tip-cost b {
    margin-left: auto;
    font-weight: 400;
  }
  .tip-cost {
    padding-top: 6px;
    margin-top: 2px;
    box-shadow: inset 0 1px 0 var(--hairline);
  }
  .tip-note {
    color: var(--ink-2);
  }
  .tip-note.rewrite {
    color: var(--warn-text);
  }
  .tip-note.agent {
    color: var(--agent-text);
  }
  .calls-note {
    margin: 4px 0 0;
  }
  .totals {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 16px 24px;
    padding-top: 16px;
    box-shadow: inset 0 1px 0 var(--hairline);
  }
  @container main (max-width: 900px) {
    .totals {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .total {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .amount {
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  .amount small {
    font-size: 12px;
    font-weight: 400;
    letter-spacing: 0;
    color: var(--ink-3);
  }
  .help {
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--ink-3);
  }
  .events {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .events li {
    display: grid;
    grid-template-columns: 62px 8px minmax(140px, 230px) minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 12px;
    padding: 10px 0;
    box-shadow: inset 0 1px 0 var(--hairline);
    font-size: 13px;
  }
  .events .time {
    color: var(--ink-3);
    white-space: nowrap;
  }
  .events .title {
    font-weight: 500;
  }
  .events .detail {
    color: var(--ink-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .events .value {
    text-align: right;
  }
  .events .value.rewrite {
    color: var(--warn-text);
  }
  .events .value.compact {
    color: var(--ink-2);
  }
  .events .value.failed {
    color: var(--critical);
  }
</style>
