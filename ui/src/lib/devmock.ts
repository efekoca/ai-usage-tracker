// Development-only IPC mock: lets the UI run in a plain browser (vite dev) with synthetic,
// deterministic data. Never included in production builds (see main.ts / widget.ts).
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
    font_family: '', text_scale: 1, number_scale: 1, number_weight: 700, tabular_nums: true,
  },
  capture: { codex_poll: false, codex_poll_minutes: 5, codex_path: '', statusline: false, otel: false, otel_port: 43180 },
  autostart: false,
  allow_config_updates: false,
  primary_metric: 'tokens',
  dismissed_unpriced: [],
  limit_display: 'used',
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
    { provider: 'anthropic', limit_id: '', window: 'five_hour', window_minutes: 300, used_pct: 46, resets_at: Math.round(now / 1000 + 2.3 * 3600), observed_ms: now - 192 * 60000, source: 'cowork_audit', status: 'allowed', plan: null, accuracy: 'exact', state: 'behind', window_start_ms: now - 2.7 * 3600e3, window_usage: totals(41e6, 31.2), usage_since: totals(190e6, 65.9), projects: [share('demo-app', 0.62, 46), share('notes', 0.38, 46)] },
    { provider: 'anthropic', limit_id: '', window: 'seven_day', window_minutes: 10080, used_pct: 73, resets_at: Math.round(now / 1000 + 2.6 * 86400), observed_ms: now - 4 * 60000, source: 'cowork_audit', status: 'allowed_warning', plan: null, accuracy: 'exact', state: 'fresh', window_start_ms: now - 4.4 * 86400e3, window_usage: totals(612e6, 402.5), usage_since: totals(0, 0), projects: [share('demo-app', 0.5, 73), share('website', 0.3, 73), share('notes', 0.2, 73)] },
    { provider: 'openai', limit_id: 'codex', window: 'five_hour', window_minutes: 300, used_pct: 12, resets_at: Math.round(now / 1000 + 4 * 3600), observed_ms: now - 20 * 60000, source: 'codex_rollout', status: null, plan: 'plus', accuracy: 'exact', state: 'fresh', window_start_ms: now - 3600e3, window_usage: totals(5e6, 3.1), usage_since: totals(0, 0), projects: [share('data-pipeline', 1, 12)] },
    { provider: 'openai', limit_id: 'codex', window: 'seven_day', window_minutes: 10080, used_pct: 31, resets_at: null, observed_ms: now - 9 * 86400e3, source: 'codex_rollout', status: null, plan: 'plus', accuracy: 'exact', state: 'reset', window_start_ms: now - 7 * 86400e3, window_usage: totals(0, 0), usage_since: totals(0, 0), projects: [] },
  ]
}

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
          return { version: '0.1.0', data_dir: 'C:\\Users\\you\\AppData\\Local\\AIUsageTracker', pricing_origin: 'bundled', pricing_updated_at: '2026-10-02', supports_mica: false, accent_color: null, started_hidden: false }
        case 'get_settings':
          return settings
        case 'save_settings':
          Object.assign(settings, a.settings)
          return settings
        case 'scan_status':
          return { running: false, done: 0, total: 0, last_scan_ms: Date.now() - 42000, last_new_events: 0, files_seen: 122, warnings: 0, errors: 0 }
        case 'get_report':
          return report(a.period as Period)
        case 'get_limits':
          return limits()
        case 'get_widget_data': {
          const per = (tok: number, cost: number) => ({ tokens: tok, cost_usd: cost, has_unpriced: false, tools: [{ tool: 'claude_code', tokens: tok * 0.8, cost_usd: cost * 0.85 }, { tool: 'codex', tokens: tok * 0.2, cost_usd: cost * 0.15 }] })
          return { today: per(18_400_000, 12.84), days7: per(96_000_000, 71.3), month1: per(402_000_000, 288.1), limits: limits().map((l) => ({ provider: l.provider, window: l.window, used_pct: l.used_pct, state: l.state, accuracy: l.accuracy, resets_at: l.resets_at, observed_ms: l.observed_ms })), providers: ['anthropic', 'openai'], updated_ms: Date.now() - 60000 }
        }
        case 'capture_status':
          return { codex_poll: settings.capture.codex_poll, codex: { binary: 'C:/codex.exe', last_ok_ms: Date.now() - 120000, last_error: null }, codex_candidates_found: true, statusline: settings.capture.statusline, statusline_file: 'C:\Users\you\.claude\settings.json', statusline_chained: false, statusline_last_ms: Date.now() - 30000, otel: settings.capture.otel, otel_port: 43180, otel_listening: settings.capture.otel, otel_events: 42, otel_last_ms: Date.now() - 5000, otel_error: null, settings_file: 'C:\Users\you\.claude\settings.json' }
        case 'set_capture': {
          const k = a.kind as 'codex' | 'statusline' | 'otel'
          if (k === 'codex') settings.capture.codex_poll = !!a.enabled
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
        default:
          return null
      }
    },
    { shouldMockEvents: true },
  )
}
