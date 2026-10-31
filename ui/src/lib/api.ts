// Types mirror the serde output of tracker-core.
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type Tool = 'claude_code' | 'codex' | 'claude_desktop' | 'antigravity' | 'opencode'
export type Provider = 'anthropic' | 'openai' | 'google'
export type Accuracy = 'exact' | 'estimated' | 'captured'
export type SourceId = 'claude_code' | 'cowork' | 'claude_desktop' | 'codex' | 'chatgpt_desktop' | 'antigravity' | 'opencode'

export const PROVIDERS: Provider[] = ['anthropic', 'openai', 'google']

/** Whose plan a source's (or tool's) usage counts toward; `null` for detection-only sources and
 * for OpenCode, whose records do not say how a request was paid. */
export function providerOfSource(s: string): Provider | null {
  if (s === 'codex') return 'openai'
  if (s === 'antigravity') return 'google'
  if (s === 'chatgpt_desktop' || s === 'opencode') return null
  return 'anthropic'
}

export function providersOf(sources: Iterable<string>): Provider[] {
  const s = new Set<Provider>()
  for (const src of sources) {
    const p = providerOfSource(src)
    if (p) s.add(p)
  }
  return PROVIDERS.filter((p) => s.has(p))
}

/** A limit window (`kind` "limit") or today's usage (`today_tokens`, `today_cost`). */
export interface TrayItem {
  kind: 'limit' | 'forecast' | 'tokens' | 'cost' | 'sessions' | 'cache' | 'scan'
  /** For usage kinds: empty for every tool, "opencode" for OpenCode alone. */
  provider: string
  window: string
  /** Antigravity's pool ("gemini" | "3p"); empty elsewhere. */
  pool: string
  /** "today" | "days7" | "month1" for usage kinds. */
  period: string
  /** A Codex or Claude limit of this account (its digest); empty for the account in use. */
  account: string
}
export type TrayMark = 'number' | 'tool' | 'anthropic' | 'openai' | 'google' | 'opencode'
export interface TraySettings {
  show_percent: boolean
  mark: TrayMark
  tool_color: boolean
  limit: string
  title: boolean
  items: TrayItem[]
  label: 'name' | 'short' | 'none'
  value: 'app' | 'used' | 'remaining'
  reset: 'none' | 'countdown' | 'time'
  style: 'percent' | 'bar' | 'bar_percent' | 'dot'
  separator: string
  prefix: string
  suffix: string
  warn_mark: boolean
  hide_icon: boolean
}
export interface TrayPreview {
  title: string
  /** Each item's own text in `items` and in the catalog; empty without a value. */
  parts: string[]
  catalog: string[]
  /** The badge the icon would draw; null keeps the app icon. */
  icon: { text: string; logo: '' | Provider | 'opencode'; bg: string; fg: string } | null
}

export type PanelKind = 'limit' | 'provider' | 'usage' | 'forecast' | 'scan' | 'action'
/** s 1×1, w 2×1, m 2×2, l 4×1 cells. */
export type PanelSize = 's' | 'w' | 'm' | 'l'
export interface PanelModule {
  kind: PanelKind
  provider: string
  pool: string
  window: string
  period: string
  action: string
  size: PanelSize
  /** "ring" | "bar" for limits; empty elsewhere. */
  style: string
  /** A Codex or Claude limit of this account (its digest); empty for the account in use. */
  account: string
}
export interface PanelSettings {
  modules: PanelModule[]
  value: 'app' | 'used' | 'remaining'
}
export interface PanelUsage {
  tokens: number
  cost_usd: number
  sessions: number
  prompt: number
  cache_read: number
}
export interface PanelLimit {
  provider: Provider
  limit_id: string
  window: string
  used_pct: number | null
  state: LimitState
  resets_at: number | null
  forecast: Forecast | null
  /** The Codex or Claude account, while there are several. */
  account: string | null
  active: boolean
}
export interface PanelData {
  limits: PanelLimit[]
  /** Today, the last 7 and the last 30 days. */
  usage: [PanelUsage, PanelUsage, PanelUsage] | null
  /** The same periods for OpenCode alone; null while its source is off. */
  opencode: [PanelUsage, PanelUsage, PanelUsage] | null
  last_scan_ms: number | null
  scanning: boolean
  widget_visible: boolean
  update: string | null
  providers: Provider[]
  /** Signed out of the tool the limits are read through, with no current reading left. */
  signed_out: Provider[]
  /** Known accounts in order of first use, which numbers the unnamed ones. */
  accounts: KnownAccount[]
}

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
  /** Accounts as `provider:id` (`accountKey`); an empty id stands for that provider's requests no evidence ties to an account. */
  accounts?: string[]
}

/** An account the widget and the panel number the unnamed ones by. */
export interface KnownAccount {
  provider: Provider
  id: string
}

/** A Codex or Claude account. A provider has none in the list while its archive holds at most one. */
export interface Account {
  provider: Provider
  /** Digest of the account id; `null` for requests no evidence ties to an account. */
  id: string | null
  first_ms: number
  last_ms: number
  requests: number
  active: boolean
  /** The plan its newest reading names, as the provider names it (Claude: "pro", "max"). */
  read_plan: string | null
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
  cache_write_with_reads: number
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
  /** Priced only from OpenCode's model catalog, not from a vendor's page. */
  catalog_models: string[]
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
  /** Codex or Claude account, only while the archive holds more than one (`null` there: an unknown account). */
  account: string | null
  /** The provider's account in use; other accounts' views show only on the Limits page. */
  active: boolean
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
  /** Estimated part of `used_pct` that rose while this computer sent no request; current readings only. */
  unlogged_pct: number | null
}

export interface WidgetPeriod {
  tokens: number
  cost_usd: number
  has_unpriced: boolean
  tools: { tool: Tool; tokens: number; cost_usd: number; has_unpriced: boolean }[]
}

export interface WidgetData {
  today: WidgetPeriod
  days7: WidgetPeriod
  month1: WidgetPeriod
  limits: { provider: Provider; limit_id: string; window: string; used_pct: number | null; state: LimitState; accuracy: Accuracy; resets_at: number | null; observed_ms: number | null; account: string | null; active: boolean }[]
  providers: Provider[]
  /** OpenCode's source is on and it was used in the last 30 days. */
  opencode: boolean
  /** Known accounts in order of first use, which numbers the unnamed ones. */
  accounts: KnownAccount[]
  updated_ms: number
}

export interface Threshold {
  provider: Provider
  window: string
  tokens?: number | null
  cost_usd?: number | null
}

export type WidgetItemKind = 'primary' | 'cost' | 'limit_five_hour' | 'limit_seven_day' | 'opencode' | 'tools' | 'week_tokens' | 'week_cost' | 'month_cost' | 'updated'

export type AntigravityPool = 'fullest' | 'gemini' | '3p' | 'all'

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
  provider_order: Provider[]
  antigravity_pool: AntigravityPool
  /** Other Codex and Claude accounts shown beside each one's account in use. */
  accounts: string[]
  primary_period: 'today' | 'days7' | 'month1'
  primary_metric: 'tokens' | 'cost'
  limit_style: 'ring' | 'bar' | 'text'
  theme: 'system' | 'light' | 'dark'
  accent: string
  corner_radius: number
  border: boolean
  shadow: boolean
  show_labels: boolean
  show_icons: boolean
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
  hotkey: string
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
  antigravity_poll: boolean
  antigravity_poll_minutes: number
  antigravity_path: string
}

export type CaptureKind = 'claude' | 'codex' | 'antigravity' | 'statusline' | 'otel'

export interface CaptureStatus {
  claude_poll: boolean
  claude: { binary: string | null; last_ok_ms: number | null; last_error: string | null }
  claude_candidates_found: boolean
  codex_poll: boolean
  codex: { binary: string | null; last_ok_ms: number | null; last_error: string | null }
  codex_candidates_found: boolean
  antigravity_poll: boolean
  antigravity: { binary: string | null; last_ok_ms: number | null; last_error: string | null }
  antigravity_candidates_found: boolean
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
  known_sources: SourceId[]
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
  tray: TraySettings
  panel: PanelSettings
  update_check: boolean
  /** provider → #rrggbb; missing = the accent color */
  provider_colors: Partial<Record<Provider, string>>
  tint_icons: boolean
  show_icons: boolean
  /** Codex or Claude account id → the name the user gave it. */
  account_names: Record<string, string>
  /** Account id → the plan chosen for it, while its provider has several accounts. */
  account_plans: Record<string, string>
  /** Account id → monthly USD price, when it differs from the list price or the plan has none. */
  account_prices: Record<string, number>
}

export interface SourceInfo {
  id: SourceId
  found: boolean
  supported: boolean
  roots: string[]
  file_count: number
  cloud_only: boolean
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
  platform: string
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
  /** The vendor whose list price this is. */
  provider: string
  aliases?: string[]
  long_context?: (Rates & { threshold: number }) | null
  speed_multipliers?: Record<string, number>
  geo_multipliers?: Record<string, number>
  web_search_per_1k?: number | null
  /** A lower price outside the vendor's busy hours (`[start, end)` UTC, Monday to Friday). */
  off_peak?: { multiplier: number; peak_hours_utc: [number, number][] } | null
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
  ladder?: number
  session_multiple?: number
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
  /** Each account's share while the provider has several; `account` null for use no evidence ties to one. */
  accounts: AccountValue[]
}
export interface AccountValue {
  account: string | null
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

export interface BranchRow {
  project_id: number | null
  project: string
  hidden: boolean
  branch: string | null
  branch_hidden: boolean
  totals: Totals
  sessions: number
  tools: Tool[]
  first_ms: number
  last_ms: number
}
export interface Branches {
  rows: BranchRow[]
  events_with_branch: number
  events_without_branch: number
}

export interface AgentRow {
  tool: Tool
  agent: string | null
  totals: Totals
  runs: number
  sessions: number
}
export interface ToolRow {
  tool: Tool
  name: string
  calls: number
  known: number
  failed: number
  by_subagents: number
}
export interface AgentsTools {
  agents: AgentRow[]
  tools: ToolRow[]
  tool_calls: number
  filtered_by_session: boolean
}

export interface PromptShare {
  requests: number
  prompt: number
  cache_read: number
}
export interface CacheAgentRow {
  tool: Tool
  agent: string | null
  totals: Totals
  runs: number
  first: PromptShare
}
export interface SubagentCache {
  main: Totals
  subagents: Totals
  runs: number
  sessions: number
  first: PromptShare
  main_after_run: PromptShare
  main_other: PromptShare
  previous_main: PromptShare
  previous_subagents: PromptShare
  rows: CacheAgentRow[]
}

export type Tip =
  | { kind: 'cache_rebuild'; requests: number; sessions: number; tokens: number; extra_usd: number; cost_share_pct: number }
  | { kind: 'long_context'; requests: number; extra_usd: number; cost_share_pct: number; models: string[] }
  | { kind: 'fast_mode'; requests: number; extra_usd: number; cost_share_pct: number }
  | { kind: 'residency'; requests: number; extra_usd: number; cost_share_pct: number }
  | { kind: 'large_contexts'; requests: number; requests_pct: number; cost_usd: number; cost_share_pct: number; threshold: number }
  | { kind: 'tool_errors'; tool: Tool; name: string; calls: number; known?: number; failed: number; rate_pct: number }
export interface Tips {
  tips: Tip[]
  requests: number
  cost_usd: number
  tool_calls: number
}

export interface WindowRecord {
  start_ms: number | null
  resets_at_ms: number | null
  /** The reset, or earlier when a newer window started first (an early reset). */
  end_ms: number | null
  first_ms: number
  last_ms: number
  peak_pct: number
  readings: number
  full: boolean
  full_at_ms: number | null
  full_minutes: number | null
  complete: boolean
  in_progress: boolean
  plan: string | null
  local: { requests: number; tokens: number; cost_usd: number; unpriced_requests: number }
}
export interface WindowSeries {
  provider: Provider
  account: string | null
  active: boolean
  limit_id: string
  window: string
  window_minutes: number
  windows: WindowRecord[]
  /** API-equivalent a whole window held, from windows the local logs explain (an estimate). */
  capacity: { median_usd: number; min_usd: number; max_usd: number; windows: number } | null
}
export interface WindowStats {
  windows: number
  complete: number
  full: number
  full_minutes: number
  peak_complete: number | null
  peak_seen: number | null
}
export type AdviceKind = 'no_plan' | 'not_applicable' | 'insufficient' | 'upgrade' | 'at_top' | 'downgrade' | 'fits'
export interface PlanAdvice {
  provider: Provider
  /** One of several accounts the advice is for; null for the provider's account in use. */
  account: string | null
  /** The plan was taken from the readings, not chosen. */
  detected: boolean
  kind: AdviceKind
  plan: string | null
  suggested: string | null
  strong: boolean
  days: number
  observed_days: number
  five_hour: WindowStats
  weekly: WindowStats
  session_ratio: number | null
  projected_five_hour: number | null
  projected_weekly_if_same_ratio: number | null
  suggested_has_no_five_hour: boolean
  monthly_delta_usd: number | null
}
/** A Claude weekly window that started again days early while one Claude account is known. */
export interface EarlyRestart {
  at_ms: number
  resets_at_ms: number
  peak_pct: number
}
export interface LimitHistoryView {
  history: { from_ms: number; to_ms: number; series: WindowSeries[]; observed_days: Record<string, number> }
  advice: PlanAdvice[]
}

export interface HourPoint {
  hour: number
  tokens: number
  cost_usd: number
  events: number
  by_tool: Record<string, number>
}
export interface DayDetail {
  date: string
  totals: Totals
  hourly: HourPoint[]
  by_tool: Group[]
  by_model: Group[]
  by_project: Group[]
  sessions: number
  first_ms: number | null
  last_ms: number | null
  limit_peaks: { provider: Provider; window: string; peak_pct: number; at_ms: number }[]
}

export interface HotkeyStatus {
  hotkey: string
  error: string | null
  desktop: boolean
}
export interface UpdateStatus {
  current: string
  configured: boolean
  checking: boolean
  last_check_ms: number | null
  last_error: string | null
  available: { version: string; notes: string | null; date: string | null } | null
  installing: boolean
  downloaded: number
  total: number | null
}

export const api = {
  appInfo: () => invoke<AppInfo>('app_info'),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings, base?: Settings) => invoke<Settings>('save_settings', { settings, base }),
  detectSources: () => invoke<SourceInfo[]>('detect_sources'),
  scanStatus: () => invoke<ScanStatus>('scan_status'),
  rescan: () => invoke<void>('rescan'),
  parserWarnings: () => invoke<ParserWarning[]>('parser_warnings'),
  report: (period: Period, filter?: Filter) => invoke<Report>('get_report', { period, filter }),
  limits: () => invoke<LimitView[]>('get_limits'),
  accounts: () => invoke<Account[]>('get_accounts'),
  sessions: (period: Period, filter?: Filter) => invoke<Sessions>('get_sessions', { period, filter }),
  compareModels: (period: Period, filter?: Filter) => invoke<ModelCompare>('compare_models', { period, filter }),
  contextStats: (period: Period, filter?: Filter) => invoke<ContextStats>('context_stats', { period, filter }),
  planValue: () => invoke<PlanValue>('plan_value'),
  exportReport: (from: string, to: string, path: string) => invoke<void>('export_report', { from, to, path }),
  openLastReport: () => invoke<void>('open_last_report'),
  widgetData: () => invoke<WidgetData>('get_widget_data'),
  trayPreview: (tray: TraySettings) => invoke<TrayPreview>('tray_preview', { tray }),
  projects: () => invoke<ProjectRow[]>('list_projects'),
  projectsForSettings: () => invoke<ProjectRow[]>('list_projects_for_settings'),
  setProjectHidden: (id: number, hidden: boolean) => invoke<void>('set_project_hidden', { id, hidden }),
  models: () => invoke<string[]>('list_models'),
  pricing: () => invoke<{ file: PricingFile; origin: 'bundled' | 'user' }>('get_pricing'),
  savePricing: (file: PricingFile) => invoke<void>('save_pricing', { file }),
  catalogPrices: (models: string[]) => invoke<ModelPrice[]>('catalog_prices', { models }),
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
  panelData: () => invoke<PanelData>('panel_data'),
  panelAction: (action: string) => invoke<void>('panel_action', { action }),
  quit: () => invoke<void>('quit_app'),
  captureStatus: () => invoke<CaptureStatus>('capture_status'),
  setCapture: (kind: CaptureKind, enabled: boolean) => invoke<string>('set_capture', { kind, enabled }),
  setReadInterval: (kind: 'claude' | 'codex' | 'antigravity', minutes: number) => invoke<Settings>('set_read_interval', { kind, minutes }),
  installCli: (kind: CaptureKind) => invoke<string>('install_cli', { kind }),
  placeWidget: (corner: string, remember = true) => invoke<void>('place_widget', { corner, remember }),
  listFonts: () => invoke<string[]>('list_fonts'),
  branches: (period: Period, filter?: Filter) => invoke<Branches>('get_branches', { period, filter }),
  agentsTools: (period: Period, filter?: Filter) => invoke<AgentsTools>('get_agents_tools', { period, filter }),
  subagentCache: (period: Period, filter?: Filter) => invoke<SubagentCache>('get_subagent_cache', { period, filter }),
  tips: (period: Period, filter?: Filter) => invoke<Tips>('get_tips', { period, filter }),
  limitHistory: (days?: number) => invoke<LimitHistoryView>('get_limit_history', { days }),
  earlyRestart: () => invoke<EarlyRestart | null>('early_restart'),
  dayDetail: (date: string, filter?: Filter) => invoke<DayDetail>('get_day_detail', { date, filter }),
  hotkeyStatus: () => invoke<HotkeyStatus>('hotkey_status'),
  setHotkey: (hotkey: string) => invoke<HotkeyStatus>('set_hotkey', { hotkey }),
  updateStatus: () => invoke<UpdateStatus>('update_status'),
  checkUpdate: () => invoke<UpdateStatus>('check_update'),
  installUpdate: () => invoke<void>('install_update'),
}

export function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => cb(e.payload))
}
