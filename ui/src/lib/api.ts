// Typed wrappers around the Rust IPC commands. Types mirror the serde output of tracker-core.
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type Tool = 'claude_code' | 'codex' | 'claude_desktop'
export type Provider = 'anthropic' | 'openai'
export type Accuracy = 'exact' | 'estimated' | 'captured'
export type SourceId = 'claude_code' | 'cowork' | 'claude_desktop' | 'codex' | 'chatgpt_desktop'

export type Period =
  | { kind: 'today' }
  | { kind: 'days7' }
  | { kind: 'month1' }
  | { kind: 'months3' }
  | { kind: 'months6' }
  | { kind: 'year1' }
  | { kind: 'all' }
  | { kind: 'custom'; from: string; to: string }

export interface Filter {
  tools?: Tool[]
  clients?: string[]
  models?: string[]
  projects?: number[]
}

export interface Tokens {
  input: number
  cache_read: number
  cache_write: number
  cache_write_1h: number
  output: number
  reasoning: number
}

export interface Cost {
  input: number
  output: number
  cache_read: number
  cache_write: number
  web_search: number
}

export interface Totals {
  events: number
  tokens: Tokens
  total_tokens: number
  cost: Cost
  cost_usd: number
  unpriced_events: number
  unpriced_tokens: number
  cache_read_with_writes: number
  cache_savings_usd: number
}

export interface Group {
  key: string
  label: string
  hidden: boolean
  totals: Totals
}

export interface DayPoint {
  date: string
  tokens: number
  cost_usd: number
  events: number
  by_tool: Record<string, number>
  cost_by_tool: Record<string, number>
  prompt_tokens: number
  cache_read: number
  cache_write: number
  cache_savings_usd: number
}

export interface Report {
  range: { from_ms: number; to_ms: number }
  totals: Totals
  previous: Totals
  by_tool: Group[]
  by_client: Group[]
  by_model: Group[]
  by_project: Group[]
  daily: DayPoint[]
  heatmap: number[][]
  peak_day: DayPoint | null
  peak_hour: number | null
  peak_weekday: number | null
  days_in_range: number
  active_days: number
  avg_daily_tokens: number
  avg_daily_cost: number
  unpriced_models: string[]
  by_accuracy: Record<string, number>
}

export type LimitState = 'fresh' | 'reset' | 'stale' | 'behind'

export interface ProjectShare {
  project_id: number | null
  name: string
  hidden: boolean
  totals: Totals
  share: number
  estimated_pct: number | null
}

export interface LimitView {
  provider: Provider
  limit_id: string
  window: string
  window_minutes: number | null
  used_pct: number | null
  resets_at: number | null
  observed_ms: number | null
  source: string
  status: string | null
  plan: string | null
  accuracy: Accuracy
  state: LimitState
  window_start_ms: number | null
  window_usage: Totals
  usage_since: Totals
  forecast: Forecast | null
  projects: ProjectShare[]
}

export interface WidgetPeriod {
  tokens: number
  cost_usd: number
  has_unpriced: boolean
  tools: { tool: Tool; tokens: number; cost_usd: number }[]
}

export interface WidgetData {
  today: WidgetPeriod
  days7: WidgetPeriod
  month1: WidgetPeriod
  limits: { provider: Provider; window: string; used_pct: number | null; state: LimitState; accuracy: Accuracy; resets_at: number | null; observed_ms: number | null }[]
  providers: Provider[]
  updated_ms: number
}

export interface Threshold {
  provider: Provider
  window: string
  tokens?: number | null
  cost_usd?: number | null
}

export type WidgetItemKind = 'primary' | 'cost' | 'limit_five_hour' | 'limit_seven_day' | 'tools' | 'week_tokens' | 'week_cost' | 'month_cost' | 'updated'

export interface WidgetSettings {
  visible: boolean
  opacity: number
  size: 's' | 'm' | 'l'
  scale: number
  x: number | null
  y: number | null
  anchor: string
  auto_hide_fullscreen: boolean
  layout: 'horizontal' | 'vertical' | 'line'
  items: { kind: WidgetItemKind; enabled: boolean }[]
  providers: Provider[]
  primary_period: 'today' | 'days7' | 'month1'
  primary_metric: 'tokens' | 'cost'
  limit_style: 'ring' | 'bar' | 'text'
  theme: 'system' | 'light' | 'dark'
  accent: string
  corner_radius: number
  border: boolean
  shadow: boolean
  show_labels: boolean
  show_reset_time: boolean
  warn_at: number
  high_at: number
  always_on_top: boolean
  lock_position: boolean
  click_action: 'open_dashboard' | 'none'
  font_family: string
  text_scale: number
  number_scale: number
  number_weight: number
  tabular_nums: boolean
}

export interface CaptureSettings {
  codex_poll: boolean
  codex_poll_minutes: number
  codex_path: string
  statusline: boolean
  otel: boolean
  otel_port: number
  claude_poll: boolean
  claude_poll_minutes: number
  claude_path: string
}

export interface CaptureStatus {
  claude_poll: boolean
  claude: { binary: string | null; last_ok_ms: number | null; last_error: string | null }
  claude_candidates_found: boolean
  codex_poll: boolean
  codex: { binary: string | null; last_ok_ms: number | null; last_error: string | null }
  codex_candidates_found: boolean
  statusline: boolean
  statusline_file: string
  statusline_chained: boolean
  statusline_last_ms: number | null
  otel: boolean
  otel_port: number
  otel_listening: boolean
  otel_events: number
  otel_last_ms: number | null
  otel_error: string | null
  settings_file: string
}

export interface Settings {
  onboarded: boolean
  language: 'system' | 'tr' | 'en'
  theme: 'system' | 'light' | 'dark'
  enabled_sources: SourceId[]
  extra_paths: { claude_config_dirs: string[]; codex_homes: string[] }
  plans: Record<string, string>
  thresholds: Threshold[]
  hide_project_names: boolean
  currency: string
  fx_rate: number
  widget: WidgetSettings
  capture: CaptureSettings
  autostart: boolean
  allow_config_updates: boolean
  primary_metric: 'tokens' | 'cost'
  dismissed_unpriced: string[]
  limit_display: 'used' | 'remaining'
  plan_prices: Record<string, number>
  weekly_report_auto: boolean
  weekly_report_dir: string
}

export interface SourceInfo {
  id: SourceId
  found: boolean
  supported: boolean
  roots: string[]
  file_count: number
  enabled: boolean
}

export interface ScanStatus {
  running: boolean
  done: number
  total: number
  last_scan_ms: number | null
  last_new_events: number
  files_seen: number
  warnings: number
  errors: number
}

export interface AppInfo {
  version: string
  data_dir: string
  pricing_origin: 'bundled' | 'user'
  pricing_updated_at: string
  supports_mica: boolean
  accent_color: string | null
  started_hidden: boolean
  reports_dir: string
}

export interface ProjectRow {
  id: number
  path: string
  name: string
  hidden: boolean
}

export interface Rates {
  input: number
  output: number
  cache_read?: number | null
  cache_write_5m?: number | null
  cache_write_1h?: number | null
}

export interface ModelPrice extends Rates {
  id: string
  provider: Provider
  aliases?: string[]
  long_context?: (Rates & { threshold: number }) | null
  speed_multipliers?: Record<string, number>
  geo_multipliers?: Record<string, number>
  web_search_per_1k?: number | null
  verified_at?: string | null
  notes?: string | null
}

export interface PricingFile {
  schema_version: number
  updated_at: string
  currency: string
  sources: Record<string, string>
  notes: string[]
  models: ModelPrice[]
  user_aliases: Record<string, string>
}

export interface PlanDef {
  id: string
  name: string
  windows: string[]
  relative?: string
  note?: { en: string; tr: string }
  source: string
  /** Monthly list price in USD; null when the plan has no fixed price. */
  monthly_usd?: number | null
  price_note?: { en: string; tr: string }
  price_options?: number[]
  price_source?: string
}

export interface PlansFile {
  schema_version: number
  updated_at: string
  prices_verified_at?: string
  price_note?: { en: string; tr: string }
  notes: { en: string; tr: string }
  providers: Record<Provider, { label: string; shared_pool_note?: { en: string; tr: string }; plans: PlanDef[] }>
}

export interface ParserWarning {
  path: string
  parser: string
  count: number
  last: string
}

export type ForecastKind = 'fills' | 'safe' | 'idle' | 'insufficient'
export interface Forecast {
  kind: ForecastKind
  rate_per_hour: number | null
  fills_at_ms: number | null
  at_reset_pct: number | null
  basis_minutes: number
}

export interface SessionModel {
  model: string
  events: number
  total_tokens: number
  cost_usd: number
  unpriced: boolean
}
export interface SessionRow {
  session_id: string
  tool: Tool
  client: string | null
  project_id: number | null
  project: string
  hidden: boolean
  started_ms: number
  ended_ms: number
  totals: Totals
  models: SessionModel[]
  max_context: number
  avg_context: number
}
export interface Sessions {
  sessions: SessionRow[]
  events_without_session: number
}

export interface CompareTarget {
  model: string
  provider: Provider
  cost_usd: number
  delta_usd: number
  verified_at: string | null
  notes: string | null
}
export interface ModelCompare {
  basis_events: number
  basis_tokens: number
  excluded_events: number
  actual_cost_usd: number
  actual_by_model: [string, number][]
  targets: CompareTarget[]
}

export interface ContextBucket {
  from: number
  to: number | null
  requests: number
  cost_usd: number
}
export interface ContextByModel {
  model: string
  requests: number
  avg: number
  p90: number
  max: number
  threshold: number | null
  over_threshold: number
  long_context_extra_usd: number
}
export interface ContextStats {
  requests: number
  avg: number
  median: number
  p90: number
  p99: number
  max: number
  buckets: ContextBucket[]
  long_context_requests: number
  long_context_extra_usd: number
  by_model: ContextByModel[]
  daily: { date: string; requests: number; avg: number; p90: number; max: number }[]
}

export interface ProviderValue {
  provider: Provider
  cost_usd: number
  events: number
  unpriced_events: number
  cumulative: number[]
}
export interface PlanValue {
  range: { from_ms: number; to_ms: number }
  dates: string[]
  providers: ProviderValue[]
}

export const api = {
  appInfo: () => invoke<AppInfo>('app_info'),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  detectSources: () => invoke<SourceInfo[]>('detect_sources'),
  scanStatus: () => invoke<ScanStatus>('scan_status'),
  rescan: () => invoke<void>('rescan'),
  parserWarnings: () => invoke<ParserWarning[]>('parser_warnings'),
  report: (period: Period, filter?: Filter) => invoke<Report>('get_report', { period, filter }),
  limits: () => invoke<LimitView[]>('get_limits'),
  sessions: (period: Period, filter?: Filter) => invoke<Sessions>('get_sessions', { period, filter }),
  compareModels: (period: Period, filter?: Filter) => invoke<ModelCompare>('compare_models', { period, filter }),
  contextStats: (period: Period, filter?: Filter) => invoke<ContextStats>('context_stats', { period, filter }),
  planValue: () => invoke<PlanValue>('plan_value'),
  exportReport: (from: string, to: string, path: string) => invoke<void>('export_report', { from, to, path }),
  openLastReport: () => invoke<void>('open_last_report'),
  widgetData: () => invoke<WidgetData>('get_widget_data'),
  projects: () => invoke<ProjectRow[]>('list_projects'),
  projectsForSettings: () => invoke<ProjectRow[]>('list_projects_for_settings'),
  setProjectHidden: (id: number, hidden: boolean) => invoke<void>('set_project_hidden', { id, hidden }),
  models: () => invoke<string[]>('list_models'),
  pricing: () => invoke<{ file: PricingFile; origin: 'bundled' | 'user' }>('get_pricing'),
  savePricing: (file: PricingFile) => invoke<void>('save_pricing', { file }),
  resetPricing: () => invoke<void>('reset_pricing'),
  plans: () => invoke<PlansFile>('get_plans'),
  exportData: (path: string, period: Period, filter: Filter | undefined, granularity: 'events' | 'daily', format: 'csv' | 'json') =>
    invoke<number>('export_data', { path, period, filter, granularity, format }),
  backup: (path: string) => invoke<void>('backup_database', { path }),
  importDb: (path: string) => invoke<{ events: number; limits: number }>('import_database', { path }),
  wipe: () => invoke<void>('wipe_all_data'),
  openDataFolder: () => invoke<void>('open_data_folder'),
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  openMain: () => invoke<void>('open_main'),
  setWidgetVisible: (visible: boolean) => invoke<void>('set_widget_visible', { visible }),
  widgetMenu: () => invoke<void>('widget_menu'),
  quit: () => invoke<void>('quit_app'),
  captureStatus: () => invoke<CaptureStatus>('capture_status'),
  setCapture: (kind: 'claude' | 'codex' | 'statusline' | 'otel', enabled: boolean) => invoke<string>('set_capture', { kind, enabled }),
  placeWidget: (corner: string, remember = true) => invoke<void>('place_widget', { corner, remember }),
  listFonts: () => invoke<string[]>('list_fonts'),
}

export function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => cb(e.payload))
}
