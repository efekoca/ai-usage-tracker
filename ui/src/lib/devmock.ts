// Dev-only IPC mock for running the UI in a plain browser; never in production builds (see main.ts).
import { mockIPC } from '@tauri-apps/api/mocks'
import plansJson from '../../../config/plans.json'
import type { DayPoint, Group, LimitView, Period, Report, Settings, Totals } from './api'

let seed = 7
const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647)

const settings: Settings = {
  onboarded: new URLSearchParams(location.search).get('onboarding') !== '1',
  language: (new URLSearchParams(location.search).get('lang') as 'tr' | 'en') ?? 'system',
  theme: (new URLSearchParams(location.search).get('theme') as 'light' | 'dark') ?? 'system',
  enabled_sources: ['claude_code', 'cowork', 'claude_desktop', 'codex'],
  extra_paths: { claude_config_dirs: [], codex_homes: [] },
  plans: { anthropic: 'max5x', openai: 'plus' },
  thresholds: [],
  hide_project_names: false,
  currency: 'USD',
  fx_rate: 1,
  widget: {
    visible: true, opacity: 0.85, size: 'm', scale: 1, x: null, y: null, anchor: 'bottom-right', auto_hide_fullscreen: true, layout: 'horizontal',
    items: (['primary', 'cost', 'limit_five_hour', 'limit_seven_day', 'tools', 'week_tokens', 'week_cost', 'month_cost', 'updated'] as const).map((k, i) => ({ kind: k, enabled: i < 4 })),
    providers: [], primary_period: 'today', primary_metric: 'tokens', limit_style: 'ring', theme: 'system', accent: '', corner_radius: 14,
    border: true, shadow: false, show_labels: true, show_reset_time: false, warn_at: 70, high_at: 90, always_on_top: true, lock_position: false, click_action: 'open_dashboard',
    font_family: '', text_scale: 1, number_scale: 1, number_weight: 700, tabular_nums: true, hotkey: 'Ctrl+Alt+Shift+W',
  },
  capture: { codex_poll: true, codex_poll_minutes: 5, codex_path: '', statusline: false, otel: false, otel_port: 43180, claude_poll: true, claude_poll_minutes: 5, claude_path: '' },
  autostart: false,
  allow_config_updates: false,
  primary_metric: 'tokens',
  dismissed_unpriced: [],
  limit_display: 'used',
  plan_prices: {},
  weekly_report_auto: false,
  weekly_report_dir: '',
  tray: { show_percent: true, limit: 'auto' },
  update_check: true,
}

const day = 864e5
const models = [
  ['claude-opus-5', 'claude_code', 0.55, 5, 25, 0.5],
  ['gpt-5.6-sol', 'codex', 0.2, 4, 20, 0.4],
  ['claude-sonnet-5', 'claude_code', 0.12, 2, 10, 0.2],
  ['claude-opus-5-5', 'claude_code', 0.08, 4, 20, 0.2],
  ['gpt-5.6-terra', 'codex', 0.04, 2, 12, 0.2],
  ['codex-auto-review', 'codex', 0.01, 0, 0, 0],
] as const
const projects = ['demo-app', 'website', 'notes', 'data-pipeline', 'mobile-client']

function totals(tokens: number, cost: number, unpriced = false): Totals {
  const input = Math.round(tokens * 0.004)
  const output = Math.round(tokens * 0.006)
  const cw = Math.round(tokens * 0.04)
  const cr = tokens - input - output - cw
  return {
    events: Math.max(1, Math.round(tokens / 120000)),
    tokens: { input, output, cache_read: cr, cache_write: cw, cache_write_1h: cw, reasoning: Math.round(output * 0.3) },
    total_tokens: tokens,
    cost: { input: cost * 0.02, output: cost * 0.12, cache_read: cost * 0.6, cache_write: cost * 0.26, web_search: 0 },
    cost_usd: unpriced ? 0 : cost,
    unpriced_events: unpriced ? 3 : 0,
    unpriced_tokens: unpriced ? tokens : 0,
    cache_savings_usd: unpriced ? 0 : cost * 0.9,
    cache_read_with_writes: Math.round(cr * 0.9),
    cache_write_with_reads: Math.round(cw * 0.9),
  }
}

function rangeDays(p: Period): number {
  return { today: 1, days7: 7, month1: 30, months3: 91, months6: 182, year1: 365, all: 104, custom: 14 }[p.kind]
}

function report(p: Period): Report {
  seed = 11
  const n = rangeDays(p)
  const end = new Date()
  end.setHours(0, 0, 0, 0)
  const daily: DayPoint[] = []
  for (let i = n - 1; i >= 0; i--) {
    const d = new Date(end.getTime() - i * day)
    const active = rnd() > 0.35 || i === 0
    const cc = active ? Math.round(2e6 + rnd() * 30e6) : 0
    const cx = active && rnd() > 0.5 ? Math.round(rnd() * 8e6) : 0
    daily.push({
      date: `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`,
      tokens: cc + cx,
      cost_usd: cc * 0.75e-6 + cx * 0.6e-6,
      events: Math.round((cc + cx) / 120000),
      by_tool: { ...(cc ? { claude_code: cc } : {}), ...(cx ? { codex: cx } : {}) },
      cost_by_tool: { ...(cc ? { claude_code: cc * 0.75e-6 } : {}), ...(cx ? { codex: cx * 0.6e-6 } : {}) },
      prompt_tokens: Math.round((cc + cx) * 0.99),
      cache_read: Math.round((cc + cx) * (0.85 + rnd() * 0.12)),
      cache_write: Math.round((cc + cx) * 0.04),
      cache_savings_usd: (cc + cx) * 0.6e-6,
    })
  }
  const total = daily.reduce((a, d) => a + d.tokens, 0)
  const cost = daily.reduce((a, d) => a + d.cost_usd, 0)
  const cc = daily.reduce((a, d) => a + (d.by_tool.claude_code ?? 0), 0)
  const g = (key: string, label: string, share: number, unpriced = false): Group => ({ key, label, hidden: false, totals: totals(Math.round(total * share), cost * share, unpriced) })
  const heat = Array.from({ length: 7 }, (_, d) => Array.from({ length: 24 }, (_, h) => (h > 8 && h < 23 ? Math.round(rnd() * 4e6 * (d < 5 ? 1 : 0.4)) : 0)))
  return {
    range: { from_ms: end.getTime() - (n - 1) * day, to_ms: end.getTime() + day },
    totals: { ...totals(total, cost), unpriced_events: 3, unpriced_tokens: Math.round(total * 0.01) },
    previous: totals(Math.round(total * 0.82), cost * 0.85),
    by_tool: [g('claude_code', 'claude_code', cc / total), g('codex', 'codex', 1 - cc / total)],
    by_client: [g('claude_code:claude-desktop', 'claude-desktop', 0.6), g('claude_code:cowork', 'cowork', 0.15), g('codex:Codex Desktop', 'Codex Desktop', 0.25)],
    by_model: models.map(([m, , s]) => g(m, m, s, m === 'codex-auto-review')),
    by_project: projects.map((p, i) => ({ ...g(String(i + 1), p, [0.4, 0.25, 0.15, 0.12, 0.08][i]), hidden: i === 4 })),
    daily,
    heatmap: heat,
    peak_day: daily.reduce((a, b) => (b.tokens > a.tokens ? b : a)),
    peak_hour: 15,
    peak_weekday: 2,
    days_in_range: n,
    active_days: daily.filter((d) => d.events > 0).length,
    avg_daily_tokens: total / n,
    avg_daily_cost: cost / n,
    unpriced_models: ['codex-auto-review'],
    by_accuracy: { exact: 1200 },
  }
}

function limits(): LimitView[] {
  const now = Date.now()
  const share = (p: string, s: number, used: number | null) => ({ project_id: projects.indexOf(p) + 1, name: p, hidden: false, totals: totals(Math.round(s * 5e7), s * 30), share: s, estimated_pct: used === null ? null : used * s })
  return [
    { provider: 'anthropic', limit_id: '', window: 'five_hour', window_minutes: 300, used_pct: 46, resets_at: Math.round(now / 1000 + 2.3 * 3600), observed_ms: now - 192 * 60000, source: 'cowork_audit', status: 'allowed', plan: null, accuracy: 'exact', state: 'behind', window_start_ms: now - 2.7 * 3600e3, window_usage: totals(41e6, 31.2), usage_since: totals(190e6, 65.9), forecast: null, projects: [share('demo-app', 0.62, 46), share('notes', 0.38, 46)] },
    { provider: 'anthropic', limit_id: '', window: 'seven_day', window_minutes: 10080, used_pct: 73, resets_at: Math.round(now / 1000 + 2.6 * 86400), observed_ms: now - 4 * 60000, source: 'cowork_audit', status: 'allowed_warning', plan: null, accuracy: 'exact', state: 'fresh', window_start_ms: now - 4.4 * 86400e3, window_usage: totals(612e6, 402.5), usage_since: totals(0, 0), forecast: { kind: 'fills', rate_per_hour: 0.9, fills_at_ms: now + 30 * 3600e3, at_reset_pct: null, basis_minutes: 5760 }, projects: [share('demo-app', 0.5, 73), share('website', 0.3, 73), share('notes', 0.2, 73)] },
    { provider: 'openai', limit_id: 'codex', window: 'five_hour', window_minutes: 300, used_pct: 12, resets_at: Math.round(now / 1000 + 4 * 3600), observed_ms: now - 20 * 60000, source: 'codex_rollout', status: null, plan: 'plus', accuracy: 'exact', state: 'fresh', window_start_ms: now - 3600e3, window_usage: totals(5e6, 3.1), usage_since: totals(0, 0), forecast: { kind: 'safe', rate_per_hour: 4.2, fills_at_ms: null, at_reset_pct: 29, basis_minutes: 60 }, projects: [share('data-pipeline', 1, 12)] },
    { provider: 'openai', limit_id: 'codex', window: 'seven_day', window_minutes: 10080, used_pct: 31, resets_at: null, observed_ms: now - 9 * 86400e3, source: 'codex_rollout', status: null, plan: 'plus', accuracy: 'exact', state: 'reset', window_start_ms: now - 7 * 86400e3, window_usage: totals(0, 0), usage_since: totals(0, 0), forecast: null, projects: [] },
  ]
}

function mockSessions() {
  const now = Date.now()
  const models = [['claude-opus-5-5', 'claude_code'], ['claude-sonnet-5', 'claude_code'], ['gpt-5.6-terra', 'codex']] as const
  const sessions = Array.from({ length: 24 }, (_, i) => {
    const [model, tool] = models[i % 3]
    const tok = Math.round(2e6 + ((i * 7919) % 13) * 3.1e6)
    const cost = tok / 1e6 * (tool === 'codex' ? 0.6 : 0.9)
    const start = now - i * 5.3 * 3600e3
    return {
      session_id: `${(0x1a2b3c4d + i * 7777).toString(16)}-4e5f-6789-abcd-${(i * 99991).toString(16).padStart(12, '0')}`,
      tool, client: tool === 'codex' ? 'Codex Desktop' : 'claude-desktop',
      project_id: (i % 5) + 1, project: projects[i % 5], hidden: i % 5 === 4,
      started_ms: start, ended_ms: start + (12 + (i * 37) % 160) * 60000,
      totals: { ...totals(tok, cost), events: 20 + (i * 13) % 140 },
      models: [{ model, events: 20 + (i * 13) % 140, total_tokens: tok, cost_usd: cost, unpriced: false }],
      max_context: 40_000 + ((i * 7) % 11) * 31_000, avg_context: 30_000 + ((i * 5) % 9) * 14_000,
    }
  })
  return { sessions, events_without_session: 3 }
}

function mockCompare() {
  const actual = 288.1
  const ids: [string, 'anthropic' | 'openai', number][] = [['gpt-5.6-luna', 'openai', 0.07], ['claude-haiku-4-5', 'anthropic', 0.24], ['gpt-5.3-codex', 'openai', 0.38], ['claude-sonnet-5', 'anthropic', 0.48], ['gpt-5.6-terra', 'openai', 0.52], ['claude-opus-5-5', 'anthropic', 0.9], ['gpt-5.6-sol', 'openai', 1.05], ['claude-opus-5', 'anthropic', 1.21], ['claude-fable-5-1', 'anthropic', 1.63], ['claude-opus-4-1', 'anthropic', 3.6]]
  return {
    basis_events: 1180, basis_tokens: 402_000_000, excluded_events: 3, actual_cost_usd: actual,
    actual_by_model: [['claude-opus-5-5', 221.4], ['gpt-5.6-terra', 51.2], ['claude-sonnet-5', 15.5]],
    targets: ids.map(([model, provider, f]) => ({ model, provider, cost_usd: actual * f, delta_usd: actual * f - actual, verified_at: '2026-10-02', notes: null })),
  }
}

function mockContext() {
  const edges = [0, 10_000, 50_000, 100_000, 200_000, 272_000, 500_000, 1_000_000]
  const counts = [40, 210, 380, 290, 70, 22, 6, 0]
  const days = Array.from({ length: 7 }, (_, i) => {
    const d = new Date(Date.now() - (6 - i) * 86400e3)
    return { date: d.toISOString().slice(0, 10), requests: 120 + i * 11, avg: 70_000 + i * 6_000, p90: 160_000 + i * 9_000, max: 240_000 + i * 20_000 }
  })
  return {
    requests: 1018, avg: 92_400, median: 71_300, p90: 181_000, p99: 290_000, max: 612_000,
    buckets: edges.map((from, i) => ({ from, to: edges[i + 1] ?? null, requests: counts[i], cost_usd: counts[i] * 0.21 })),
    long_context_requests: 22, long_context_extra_usd: 6.84,
    by_model: [
      { model: 'claude-opus-5-5', requests: 640, avg: 98_000, p90: 190_000, max: 612_000, threshold: null, over_threshold: 0, long_context_extra_usd: 0 },
      { model: 'gpt-5.6-terra', requests: 290, avg: 88_000, p90: 230_000, max: 401_000, threshold: 272_000, over_threshold: 22, long_context_extra_usd: 6.84 },
      { model: 'claude-sonnet-5', requests: 88, avg: 41_000, p90: 90_000, max: 140_000, threshold: null, over_threshold: 0, long_context_extra_usd: 0 },
    ],
    daily: days,
  }
}

function mockPlanValue() {
  const n = 30
  const dates = Array.from({ length: n }, (_, i) => new Date(Date.now() - (n - 1 - i) * 86400e3).toISOString().slice(0, 10))
  const run = (per: number) => { let r = 0; return dates.map((_, i) => (r += per * (0.4 + ((i * 37) % 10) / 8))) }
  const a = run(9.1)
  const o = run(1.2)
  return {
    range: { from_ms: Date.now() - n * 86400e3, to_ms: Date.now() },
    dates,
    providers: [
      { provider: 'anthropic', cost_usd: a[n - 1], events: 9000, unpriced_events: 0, cumulative: a },
      { provider: 'openai', cost_usd: o[n - 1], events: 1200, unpriced_events: 93, cumulative: o },
    ],
  }
}

function mockBranches() {
  const row = (pid: number, branch: string | null, share: number, sessions: number, tools: string[], ago: number) => ({
    project_id: pid, project: pid === 5 ? '' : projects[pid - 1], hidden: pid === 5, branch: pid === 5 ? null : branch, branch_hidden: pid === 5 && branch !== null, totals: totals(Math.round(402_000_000 * share), 288 * share), sessions, tools,
    first_ms: Date.now() - (ago + 3) * day, last_ms: Date.now() - ago * day,
  })
  return {
    rows: [
      row(1, 'main', 0.31, 9, ['claude_code', 'codex'], 0),
      row(1, 'feature/limit-history', 0.18, 4, ['claude_code'], 0.2),
      row(2, 'main', 0.14, 6, ['claude_code'], 1),
      row(3, null, 0.11, 7, ['claude_code'], 2),
      row(1, 'fix/tray-icon', 0.08, 2, ['codex'], 3),
      row(4, 'develop', 0.07, 3, ['claude_code'], 5),
      row(5, 'main', 0.04, 1, ['claude_code'], 9),
    ],
    events_with_branch: 1120,
    events_without_branch: 160,
  }
}

function mockAgentsTools() {
  const t = (tool: string, name: string, calls: number, known: number, failed: number, sub: number) => ({ tool, name, calls, known, failed, by_subagents: sub })
  return {
    agents: [
      { tool: 'claude_code', agent: null, totals: totals(310_000_000, 251.2), runs: 0, sessions: 31 },
      { tool: 'claude_code', agent: 'general-purpose', totals: totals(28_000_000, 19.4), runs: 14, sessions: 6 },
      { tool: 'claude_code', agent: 'Explore', totals: totals(9_000_000, 4.1), runs: 9, sessions: 4 },
      { tool: 'codex', agent: null, totals: totals(52_000_000, 13.3), runs: 0, sessions: 12 },
      { tool: 'codex', agent: 'guardian', totals: { ...totals(1_200_000, 0, true), events: 40, unpriced_events: 40 }, runs: 5, sessions: 3 },
    ],
    tools: [
      t('claude_code', 'Bash', 1180, 1176, 41, 160), t('claude_code', 'Edit', 402, 402, 3, 0), t('claude_code', 'Read', 360, 358, 6, 120),
      t('codex', 'shell', 310, 310, 84, 0), t('claude_code', 'Write', 141, 141, 2, 0), t('claude_code', 'mcp__Claude_Browser__navigate', 96, 96, 9, 0),
      t('codex', 'apply_patch', 88, 88, 1, 0), t('claude_code', 'Grep', 64, 64, 1, 30), t('claude_code', 'WebSearch', 37, 37, 0, 21),
      t('codex', 'mcp__node_repl__js', 33, 33, 4, 0), t('codex', 'web_search', 19, 0, 0, 0), t('claude_code', 'TaskUpdate', 17, 17, 0, 0),
    ],
    tool_calls: 2747,
    filtered_by_session: false,
  }
}

function mockTips() {
  return {
    tips: [
      { kind: 'cache_rebuild', requests: 9, sessions: 6, tokens: 1_480_000, extra_usd: 11.42, cost_share_pct: 4.0 },
      { kind: 'long_context', requests: 22, extra_usd: 6.84, cost_share_pct: 2.4, models: ['gpt-5.6-terra'] },
      { kind: 'large_contexts', requests: 840, requests_pct: 71.2, cost_usd: 241.3, cost_share_pct: 83.8, threshold: 100_000 },
      { kind: 'tool_errors', tool: 'codex', name: 'shell', calls: 310, failed: 84, rate_pct: 27.1 },
    ],
    requests: 1180,
    cost_usd: 288.1,
    tool_calls: 2747,
  }
}

function mockHistory() {
  const now = Date.now()
  const H = 3600e3
  const five = Array.from({ length: 24 }, (_, i) => {
    const reset = now - (27 - i) * day + ((i * 7) % 10) * H
    const peak = [18, 44, 63, 100, 37, 22, 81, 55, 12, 100, 47, 28, 66, 39, 91, 24, 51, 33, 70, 15, 58, 42, 100, 31][i]
    return { start_ms: reset - 5 * H, resets_at_ms: reset, end_ms: reset, first_ms: reset - 4.5 * H, last_ms: reset - (i % 4 === 1 ? 2 * H : 0.2 * H), peak_pct: peak, readings: 30, full: peak >= 99.5, full_at_ms: peak >= 99.5 ? reset - H : null, full_minutes: peak >= 99.5 ? 60 : null, complete: peak >= 99.5 || i % 4 !== 1, in_progress: false, plan: null }
  })
  five[five.length - 1] = { ...five[five.length - 1], resets_at_ms: now + 2 * H, end_ms: now + 2 * H, start_ms: now - 3 * H, last_ms: now - 0.1 * H, in_progress: true, complete: false, full: false, full_at_ms: null, full_minutes: null, peak_pct: 31 }
  const week = Array.from({ length: 8 }, (_, i) => {
    const reset = now - (7 - i) * 7 * day + 2 * day
    const peak = [34, 52, 61, 100, 47, 58, 72, 41][i]
    return { start_ms: reset - 7 * day, resets_at_ms: reset, end_ms: reset, first_ms: reset - 6.8 * day, last_ms: reset - (i === 2 ? 2 * day : 0.3 * day), peak_pct: peak, readings: 400, full: peak >= 99.5, full_at_ms: peak >= 99.5 ? reset - 9 * H : null, full_minutes: peak >= 99.5 ? 540 : null, complete: i !== 2, in_progress: i === 7, plan: null }
  })
  const codexWeek = week.map((w, i) => ({ ...w, peak_pct: [12, 20, 9, 31, 26, 18, 44, 7][i], full: false, full_at_ms: null, full_minutes: null, plan: 'plus' }))
  const codexFive = five.slice(0, 12).map((w, i) => ({ ...w, peak_pct: [8, 22, 15, 41, 5, 13, 29, 18, 9, 36, 12, 7][i], full: false, full_at_ms: null, full_minutes: null, plan: 'plus', complete: i % 3 !== 0, in_progress: false }))
  const withLocal = <T extends { peak_pct: number }>(list: T[], perPct: number) =>
    list.map((w, i) => ({ ...w, local: { requests: Math.round(w.peak_pct * 9 + i), tokens: Math.round(w.peak_pct * 2.1e6), cost_usd: w.peak_pct * perPct * (0.85 + ((i * 7) % 5) / 15), unpriced_requests: 0 } }))
  const stats = (w: number, c: number, f: number, fm: number, pc: number | null, ps: number | null) => ({ windows: w, complete: c, full: f, full_minutes: fm, peak_complete: pc, peak_seen: ps })
  return {
    history: {
      from_ms: now - 56 * day,
      to_ms: now,
      series: [
        { provider: 'anthropic', limit_id: '', window: 'five_hour', window_minutes: 300, windows: withLocal(five, 0.21), capacity: { median_usd: 21.4, min_usd: 17.9, max_usd: 26.2, windows: 14 } },
        { provider: 'anthropic', limit_id: '', window: 'seven_day', window_minutes: 10080, windows: withLocal(week, 1.6), capacity: { median_usd: 163, min_usd: 139, max_usd: 181, windows: 5 } },
        { provider: 'openai', limit_id: 'codex', window: 'five_hour', window_minutes: 300, windows: withLocal(codexFive, 0.05), capacity: { median_usd: 4.9, min_usd: 3.8, max_usd: 6.1, windows: 6 } },
        { provider: 'openai', limit_id: 'codex', window: 'seven_day', window_minutes: 10080, windows: withLocal(codexWeek, 0.3), capacity: null },
      ],
      observed_days: { anthropic: 26, openai: 14 },
    },
    advice: [
      { provider: 'anthropic', kind: 'upgrade', plan: 'max5x', suggested: 'max20x', strong: false, days: 28, observed_days: 26, five_hour: stats(24, 18, 3, 180, 100, 100), weekly: stats(4, 3, 1, 540, 100, 100), session_ratio: 4, projected_five_hour: null, projected_weekly_if_same_ratio: null, suggested_has_no_five_hour: false, monthly_delta_usd: 100 },
      { provider: 'openai', kind: 'fits', plan: 'plus', suggested: null, strong: false, days: 28, observed_days: 14, five_hour: stats(12, 8, 0, 0, 41, 41), weekly: stats(4, 3, 0, 0, 44, 44), session_ratio: null, projected_five_hour: null, projected_weekly_if_same_ratio: null, suggested_has_no_five_hour: false, monthly_delta_usd: null },
    ],
    detected_plans: {},
  }
}

function mockDay(date: string) {
  const seedDay = Number(date.slice(-2))
  const hourly = Array.from({ length: 24 }, (_, h) => {
    const on = h >= 9 && h <= 23 && (h + seedDay) % 5 !== 0
    const cc = on ? Math.round((1 + ((h * 13 + seedDay) % 7)) * 1.9e6) : 0
    const cx = on && h % 3 === 0 ? Math.round(6e5 + (h % 4) * 2e5) : 0
    return { hour: h, tokens: cc + cx, cost_usd: cc * 0.75e-6 + cx * 0.6e-6, events: Math.round((cc + cx) / 120000), by_tool: { ...(cc ? { claude_code: cc } : {}), ...(cx ? { codex: cx } : {}) } }
  })
  const tok = hourly.reduce((a, h) => a + h.tokens, 0)
  const cost = hourly.reduce((a, h) => a + h.cost_usd, 0)
  const g = (key: string, label: string, share: number, hidden = false) => ({ key, label, hidden, totals: totals(Math.round(tok * share), cost * share) })
  return {
    date,
    totals: { ...totals(tok, cost), events: hourly.reduce((a, h) => a + h.events, 0) },
    hourly,
    by_tool: [g('claude_code', 'claude_code', 0.86), g('codex', 'codex', 0.14)],
    by_model: [g('claude-opus-5-5', 'claude-opus-5-5', 0.62), g('claude-sonnet-5', 'claude-sonnet-5', 0.24), g('gpt-5.6-terra', 'gpt-5.6-terra', 0.14)],
    by_project: [g('1', 'demo-app', 0.55), g('2', 'website', 0.3), g('5', '', 0.15, true)],
    sessions: 6,
    first_ms: new Date(date + 'T09:12:00').getTime(),
    last_ms: new Date(date + 'T23:41:00').getTime(),
    limit_peaks: [
      { provider: 'anthropic', window: 'five_hour', peak_pct: seedDay % 4 === 0 ? 100 : 71, at_ms: new Date(date + 'T16:20:00').getTime() },
      { provider: 'anthropic', window: 'seven_day', peak_pct: 44, at_ms: new Date(date + 'T23:30:00').getTime() },
      { provider: 'openai', window: 'five_hour', peak_pct: 18, at_ms: new Date(date + 'T15:05:00').getTime() },
    ],
  }
}

const update = { current: '0.2.6', configured: true, checking: false, last_check_ms: Date.now() - 3600e3, last_error: null, available: null as null | { version: string; notes: string | null; date: string | null }, installing: false, downloaded: 0, total: null }

const pricing = {
  schema_version: 1,
  updated_at: '2026-10-02',
  currency: 'USD',
  sources: { anthropic: 'https://platform.claude.com/docs/en/about-claude/pricing', openai: 'https://developers.openai.com/api/docs/pricing' },
  notes: [],
  models: models.filter((m) => m[3] > 0).map(([id, tool, , i, o, cr]) => ({ id, provider: tool === 'codex' ? 'openai' : 'anthropic', input: i, output: o, cache_read: cr, cache_write_5m: i * 1.25, cache_write_1h: tool === 'codex' ? null : i * 2, verified_at: '2026-10-02' })),
  user_aliases: {},
}

export function installMock() {
  mockIPC(
    (cmd, args) => {
      const a = args as Record<string, unknown>
      switch (cmd) {
        case 'app_info':
          return { version: '0.2.6', data_dir: 'C:\\Users\\you\\AppData\\Local\\AIUsageTracker', pricing_origin: 'bundled', pricing_updated_at: '2026-10-02', supports_mica: false, accent_color: null, started_hidden: false, reports_dir: 'C:\\Users\\you\\Documents\\AI Usage Tracker' }
        case 'get_settings':
          return settings
        case 'save_settings':
          Object.assign(settings, a.settings)
          return settings
        case 'scan_status':
          return { running: false, done: 0, total: 0, last_scan_ms: Date.now() - 42000, last_new_events: 0, files_seen: 122, warnings: 0, errors: 0 }
        case 'get_report':
          return report(a.period as Period)
        case 'get_sessions':
          return mockSessions()
        case 'compare_models':
          return mockCompare()
        case 'context_stats':
          return mockContext()
        case 'plan_value':
          return mockPlanValue()
        case 'get_limits':
          return limits()
        case 'get_widget_data': {
          const per = (tok: number, cost: number) => ({ tokens: tok, cost_usd: cost, has_unpriced: false, tools: [{ tool: 'claude_code', tokens: tok * 0.8, cost_usd: cost * 0.85 }, { tool: 'codex', tokens: tok * 0.2, cost_usd: cost * 0.15 }] })
          return { today: per(18_400_000, 12.84), days7: per(96_000_000, 71.3), month1: per(402_000_000, 288.1), limits: limits().map((l) => ({ provider: l.provider, window: l.window, used_pct: l.used_pct, state: l.state, accuracy: l.accuracy, resets_at: l.resets_at, observed_ms: l.observed_ms })), providers: ['anthropic', 'openai'], updated_ms: Date.now() - 60000 }
        }
        case 'capture_status':
          return { claude_poll: settings.capture.claude_poll, claude: { binary: 'C:/claude.exe', last_ok_ms: Date.now() - 40000, last_error: null }, claude_candidates_found: true, codex_poll: settings.capture.codex_poll, codex: { binary: 'C:/codex.exe', last_ok_ms: Date.now() - 120000, last_error: null }, codex_candidates_found: true, statusline: settings.capture.statusline, statusline_file: 'C:\\Users\\you\\.claude\\settings.json', statusline_chained: false, statusline_last_ms: Date.now() - 30000, otel: settings.capture.otel, otel_port: 43180, otel_listening: settings.capture.otel, otel_events: 42, otel_last_ms: Date.now() - 5000, otel_error: null, settings_file: 'C:\\Users\\you\\.claude\\settings.json' }
        case 'set_capture': {
          const k = a.kind as 'claude' | 'codex' | 'statusline' | 'otel'
          if (k === 'codex') settings.capture.codex_poll = !!a.enabled
          else if (k === 'claude') settings.capture.claude_poll = !!a.enabled
          else settings.capture[k] = !!a.enabled
          return a.enabled ? 'enabled' : 'restored'
        }
        case 'place_widget':
          return null
        case 'list_fonts':
          return ['Arial', 'Bahnschrift', 'Calibri', 'Cascadia Mono', 'Consolas', 'Georgia', 'Segoe UI', 'Segoe UI Variable Display', 'Times New Roman', 'Verdana']
        case 'detect_sources':
          return [
            { id: 'claude_code', found: true, supported: true, roots: ['C:\\Users\\you\\.claude'], file_count: 27, enabled: true },
            { id: 'cowork', found: true, supported: true, roots: ['C:\\Users\\you\\AppData\\Roaming\\Claude\\local-agent-mode-sessions'], file_count: 39, enabled: true },
            { id: 'claude_desktop', found: true, supported: true, roots: ['C:\\Users\\you\\AppData\\Roaming\\Claude'], file_count: 1, enabled: true },
            { id: 'codex', found: true, supported: true, roots: ['C:\\Users\\you\\.codex'], file_count: 55, enabled: true },
            { id: 'chatgpt_desktop', found: false, supported: false, roots: [], file_count: 0, enabled: false },
          ]
        case 'parser_warnings':
          return []
        case 'list_projects':
        case 'list_projects_for_settings':
          return projects.map((p, i) => ({ id: i + 1, name: p, path: `C:\\Projects\\${p}`, hidden: i === 4 }))
        case 'list_models':
          return models.map((m) => m[0])
        case 'get_pricing':
          return { file: pricing, origin: 'bundled' }
        case 'get_plans':
          return plansJson
        case 'get_branches':
          return mockBranches()
        case 'get_agents_tools':
          return mockAgentsTools()
        case 'get_tips':
          return mockTips()
        case 'get_limit_history':
          return mockHistory()
        case 'get_day_detail':
          return mockDay(String(a.date))
        case 'hotkey_status':
          return { hotkey: settings.widget.hotkey, error: null }
        case 'set_hotkey':
          settings.widget.hotkey = String(a.hotkey)
          return { hotkey: settings.widget.hotkey, error: null }
        case 'update_status':
          return update
        case 'check_update':
          update.last_check_ms = Date.now()
          update.available = new URLSearchParams(location.search).get('update') === '1' ? { version: '0.3.0', notes: 'Bug fixes', date: null } : null
          return update
        default:
          return null
      }
    },
    { shouldMockEvents: true },
  )
}
