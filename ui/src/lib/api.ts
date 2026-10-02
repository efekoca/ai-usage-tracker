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

export type LimitState = 'fresh' | 'reset' | 'stale'

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
  projects: ProjectShare[]
}

export interface WidgetData {
  total_tokens: number
  cost_usd: number
  has_unpriced: boolean
  tools: { tool: Tool; tokens: number; cost_usd: number }[]
  limits: { provider: Provider; window: string; used_pct: number | null; state: LimitState; accuracy: Accuracy; resets_at: number | null }[]
  providers: Provider[]
}

export interface Threshold {
  provider: Provider
  window: string
  tokens?: number | null
  cost_usd?: number | null
}

export interface WidgetSettings {
  visible: boolean
  opacity: number
  size: 's' | 'm' | 'l'
  x: number | null
  y: number | null
  auto_hide_fullscreen: boolean
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
  autostart: boolean
  allow_config_updates: boolean
  primary_metric: 'tokens' | 'cost'
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
}

export interface PlansFile {
  schema_version: number
  updated_at: string
  notes: { en: string; tr: string }
  providers: Record<Provider, { label: string; shared_pool_note?: { en: string; tr: string }; plans: PlanDef[] }>
}

export interface ParserWarning {
  path: string
  parser: string
  count: number
  last: string
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
}

export function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => cb(e.payload))
}
