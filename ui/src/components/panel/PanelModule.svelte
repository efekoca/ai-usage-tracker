<script lang="ts" module>
  import type { PanelData, PanelLimit, PanelModule, Provider } from '../../lib/api'

  /** The module's limit window; Antigravity's other pool is any pool that is not Gemini. */
  /** A module without an account follows the account in use. */
  export function findLimit(data: PanelData | null, provider: string, pool: string, window: string, account = ''): PanelLimit | null {
    const mine = (l: PanelLimit) => (account ? l.account === account : l.active)
    const all = (data?.limits ?? []).filter((l) => l.provider === provider && l.window === window && mine(l) && (provider !== 'google' || (pool === 'gemini') === (l.limit_id === 'gemini')))
    return all.sort((a, b) => (b.used_pct ?? -1) - (a.used_pct ?? -1))[0] ?? null
  }

  export const live = (l: PanelLimit | null): l is PanelLimit & { used_pct: number } => !!l && l.used_pct !== null && (l.state === 'fresh' || l.state === 'behind')

  export const moduleKey = (m: PanelModule) => `${m.kind}:${m.provider}:${m.pool}:${m.window}:${m.period}:${m.action}:${m.account}`

  export const providerOf = (m: PanelModule) => m.provider as Provider
</script>

<script lang="ts">
  import type { Settings } from '../../lib/api'
  import { fmtClock, fmtCompact, fmtMoney, fmtPct, fmtSpan, fmtTime, limitName, limitShown, t, toolLabel, windowLabel, type LimitMode } from '../../lib/i18n.svelte'
  import { nameAccount } from '../../lib/store.svelte'
  import BrandIcon from '../BrandIcon.svelte'
  import Icon from '../Icon.svelte'
  import Ring from './Ring.svelte'

  let {
    m,
    data,
    settings,
    now,
    mode,
    scanned = false,
    edit = false,
    onaction,
  }: { m: PanelModule; data: PanelData | null; settings: Settings; now: number; mode: LimitMode; scanned?: boolean; edit?: boolean; onaction?: (action: string) => void } = $props()
  const warnAt = $derived(settings.widget.warn_at)
  const highAt = $derived(settings.widget.high_at)
  const color = (used: number | null, p: string) =>
    used !== null && used >= highAt ? 'var(--critical)' : used !== null && used >= warnAt ? 'var(--serious)' : settings.provider_colors?.[p as Provider] || 'var(--accent)'
  // another Codex account's tile names it, and then the account in use is named too
  const account = $derived.by(() => {
    if (m.provider !== 'openai') return null
    if (m.account) return m.account
    return settings.panel.modules.some((x) => x.account) ? (data?.limits.find((l) => l.provider === 'openai' && l.active)?.account ?? null) : null
  })
  // Antigravity's pools and Codex accounts by their short names, as on the widget; the logo stands beside them
  const name = $derived(
    m.provider === 'google'
      ? t(`widget.pool.google.${m.pool === 'gemini' ? 'gemini' : '3p'}`)
      : account
        ? nameAccount(account, data?.accounts ?? [], settings.account_names, true)
        : limitName(m.provider, m.pool),
  )
  const short = (w: string) => t(`panel.window.${w}`)

  type Shown = { value: number | null; used: number | null; text: string; sub: string; why: string; behind: boolean }
  function shown(l: PanelLimit | null): Shown {
    if (live(l)) {
      const v = limitShown(l.used_pct, mode)
      return { value: v, used: l.used_pct, text: fmtPct(v), sub: t(`panel.${mode}`), why: '', behind: l.state === 'behind' }
    }
    const k = !l ? 'none' : l.state === 'reset' ? 'reset' : 'stale'
    return { value: null, used: null, text: '—', sub: t(`panel.why.${k}`), why: t(`panel.short.${k}`), behind: false }
  }
  // a time as what is left until it ("2h 14m"): short enough for a narrow tile in any language,
  // where a clock time like "Wed 11:59 PM" does not fit
  function until(atMs: number): string {
    const total = Math.ceil((atMs - now) / 60000)
    if (total <= 0) return ''
    const [d, h, m] = [Math.floor(total / 1440), Math.floor((total % 1440) / 60), total % 60]
    return d ? t('panel.left.dh', { d, h }) : h ? t('panel.left.hm', { h, m }) : t('panel.left.m', { m })
  }
  function resetLeft(l: PanelLimit | null): string {
    return live(l) && l.resets_at ? until(l.resets_at * 1000) : ''
  }
  function resetAt(l: PanelLimit | null): string {
    if (!live(l) || !l.resets_at || l.resets_at * 1000 <= now) return ''
    const at = l.resets_at * 1000
    return at - now < 86_400_000 ? fmtClock(at) : fmtTime(at)
  }
  function forecast(l: PanelLimit | null): { text: string; at?: string; short: string; tone: 'bad' | 'ok' | 'none' } {
    const f = live(l) ? l.forecast : null
    if (f?.kind === 'fills' && f.fills_at_ms) {
      const at = f.fills_at_ms - now < 86_400_000 ? fmtClock(f.fills_at_ms) : fmtTime(f.fills_at_ms)
      return { text: t('panel.forecast.fills', { t: until(f.fills_at_ms) }), at: t('panel.forecast.fillsAt', { t: at }), short: `⇡ ${until(f.fills_at_ms)}`, tone: 'bad' }
    }
    if ((f?.kind === 'safe' || f?.kind === 'idle') && f.at_reset_pct !== null) {
      const pct = fmtPct(limitShown(f.at_reset_pct, mode))
      return { text: t('panel.forecast.safe', { pct }), short: `→ ${pct}`, tone: 'ok' }
    }
    return { text: t('panel.forecast.none'), short: '', tone: 'none' }
  }

  const one = $derived(m.kind === 'limit' || m.kind === 'forecast' ? findLimit(data, m.provider, m.pool, m.window, m.account) : null)
  const both = $derived(m.kind === 'provider' ? (['five_hour', 'seven_day'] as const).map((w) => ({ w, l: findLimit(data, m.provider, m.pool, w, m.account) })) : [])
  const oc = $derived(m.kind === 'usage' && m.provider === 'opencode')
  const usage = $derived.by(() => {
    const set = m.kind !== 'usage' ? null : oc ? data?.opencode : data?.usage
    return set ? set[['today', 'days7', 'month1'].indexOf(m.period)] : null
  })
  const hit = $derived(usage && usage.prompt > 0 ? (usage.cache_read / usage.prompt) * 100 : null)

  const actionIcon: Record<string, string> = { open: 'overview', widget: 'widget', rescan: 'refresh', settings: 'settings' }
  const active = $derived(m.action === 'widget' ? !!data?.widget_visible : m.action === 'rescan' ? !!data?.scanning : false)
  const scanning = $derived(!!data?.scanning)
  const actionLabel = $derived(
    m.action === 'widget' ? t(data?.widget_visible ? 'panel.action.widget.on' : 'panel.action.widget.off') : m.action === 'rescan' && scanning ? t('panel.scan.running') : m.action === 'rescan' && scanned ? t('panel.scan.done') : t(`panel.action.${m.action}`),
  )
  const act = (a: string) => () => !edit && onaction?.(a)
</script>

{#snippet logo(size: number)}
  <span class="logo"><BrandIcon provider={providerOf(m)} {size} always /></span>
{/snippet}

{#snippet bar(s: Shown, p: string)}
  <span class="track"><span class="fill" style:width="{s.value ?? 0}%" style:background={color(s.used, p)}></span></span>
{/snippet}

{#if m.kind === 'action'}
  <button class="mod action {m.size}" class:on={active} class:spin={m.action === 'rescan' && scanning} class:done={m.action === 'rescan' && scanned && !scanning} title={actionLabel} aria-label={actionLabel} aria-pressed={m.action === 'widget' ? active : undefined} aria-busy={m.action === 'rescan' ? scanning : undefined} disabled={m.action === 'rescan' && scanning && !edit} tabindex={edit ? -1 : 0} onclick={act(m.action)}>
    <span class="disc"><Icon name={m.action === 'rescan' && scanned && !scanning ? 'check' : actionIcon[m.action]} size={16} /></span>
    {#if m.size === 'w'}<span class="lbl">{actionLabel}</span>{/if}
  </button>
{:else if m.kind === 'scan'}
  <button class="mod scan w" class:spin={scanning} class:done={scanned && !scanning} aria-busy={scanning} disabled={scanning && !edit} tabindex={edit ? -1 : 0} onclick={act('rescan')} title={t('panel.action.rescan')}>
    <span class="disc"><Icon name={scanned && !scanning ? 'check' : 'refresh'} size={16} /></span>
    <span class="txt">
      <span class="lbl">{t('panel.scan')}</span>
      <span class="sub">{scanning ? t('panel.scan.running') : scanned ? t('panel.scan.done') : data?.last_scan_ms ? t('panel.scan.ago', { span: fmtSpan(Math.max(0, now - data.last_scan_ms)) }) : t('panel.scan.never')}</span>
    </span>
  </button>
{:else if m.kind === 'limit'}
  {@const s = shown(one)}
  <div class="mod limit {m.size} {m.style}" role="group" aria-label="{name} · {windowLabel(m.window)}: {s.text} {s.sub}">
    {#if m.size === 's'}
      {#if m.style === 'bar'}
        <span class="vfill" style:height="{s.value ?? 0}%" style:background={color(s.used, m.provider)}></span>
        <span class="stack">
          <b class="num">{s.text}</b>
          <span class="tiny">{short(m.window)}</span>
        </span>
      {:else}
        <Ring value={s.value} color={color(s.used, m.provider)} size={40} stroke={4}><b class="num small">{s.value === null ? '—' : Math.round(s.value)}</b></Ring>
        <span class="tiny">{short(m.window)}</span>
      {/if}
    {:else if m.size === 'w'}
      {#if m.style === 'bar'}
        <span class="txt grow">
          <span class="row"><span class="lbl">{name}</span><b class="num val">{s.behind ? '~' : ''}{s.text}</b></span>
          {@render bar(s, m.provider)}
          <span class="sub" title={resetAt(one) || s.sub}>{short(m.window)}{resetLeft(one) ? ` · ↻ ${resetLeft(one)}` : s.value === null ? ` · ${s.why}` : ''}</span>
        </span>
      {:else}
        <Ring value={s.value} color={color(s.used, m.provider)} size={38} stroke={4}>{@render logo(13)}</Ring>
        <span class="txt">
          <span class="lbl">{name}</span>
          <span class="row start"><b class="num val">{s.behind ? '~' : ''}{s.text}</b><span class="unit">{short(m.window)}</span></span>
          <span class="sub" title={resetAt(one) || s.sub}>{resetLeft(one) ? `↻ ${resetLeft(one)}` : s.value === null ? s.why : ' '}</span>
        </span>
      {/if}
    {:else}
      {@const f = forecast(one)}
      <span class="head">{@render logo(14)}<span class="lbl">{name}</span></span>
      {#if m.style === 'bar'}
        <span class="big">
          <b class="num hero">{s.behind ? '~' : ''}{s.text}</b>
          <span class="sub" title={s.sub}>{s.value === null ? s.why : s.sub} · {short(m.window)}</span>
          {@render bar(s, m.provider)}
        </span>
      {:else}
        <span class="center">
          <Ring value={s.value} color={color(s.used, m.provider)} size={70} stroke={6.5}>
            <b class="num mid">{s.behind ? '~' : ''}{s.text}</b>
            <span class="tiny">{s.value === null ? '' : s.sub}</span>
          </Ring>
        </span>
      {/if}
      <span class="foot">
        <span class="sub" title={resetAt(one) || undefined}>{resetLeft(one) ? `↻ ${resetLeft(one)}` : short(m.window)}</span>
        {#if f.short}<span class="sub" class:bad={f.tone === 'bad'} title={f.at ?? f.text}>{f.short}</span>{/if}
      </span>
    {/if}
  </div>
{:else if m.kind === 'provider'}
  {@const rows = both.map((b) => ({ ...b, s: shown(b.l) }))}
  <div class="mod provider {m.size} {m.style}" role="group" aria-label="{name}: {rows.map((r) => `${windowLabel(r.w)} ${r.s.text}`).join(', ')}">
    {#if m.size === 'm' && m.style !== 'bar'}
      <span class="head">{@render logo(14)}<span class="lbl">{name}</span></span>
      <span class="pair">
        {#each rows as r (r.w)}
          <span class="cell">
            <Ring value={r.s.value} color={color(r.s.used, m.provider)} size={50} stroke={5}><b class="num small">{r.s.value === null ? '—' : fmtPct(r.s.value)}</b></Ring>
            <span class="tiny">{short(r.w)}</span>
            <span class="tiny dim fit" title={r.s.value === null ? r.s.sub : resetAt(r.l) || undefined}>{resetLeft(r.l) ? `↻ ${resetLeft(r.l)}` : r.s.why || ' '}</span>
          </span>
        {/each}
      </span>
    {:else}
      <span class="head">{@render logo(14)}<span class="lbl">{name}</span>{#if m.size === 'l'}<span class="sub push">{rows.some((r) => r.s.behind) ? t('panel.behind') : ''}</span>{/if}</span>
      <span class="bars" class:side={m.size === 'l'}>
        {#each rows as r (r.w)}
          <span class="brow">
            <span class="tiny key">{short(r.w)}</span>
            {@render bar(r.s, m.provider)}
            <b class="num tiny val">{r.s.behind ? '~' : ''}{r.s.text}</b>
            {#if m.size !== 'w'}<span class="tiny dim when fit" title={resetAt(r.l) || undefined}>{resetLeft(r.l) ? `↻ ${resetLeft(r.l)}` : r.s.why}</span>{/if}
          </span>
        {/each}
      </span>
    {/if}
  </div>
{:else if m.kind === 'forecast'}
  {@const f = forecast(one)}
  {@const s = shown(one)}
  <div class="mod forecast {m.size}" role="group" aria-label="{name} · {windowLabel(m.window)} · {t('panel.forecast')}: {f.text}">
    {#if m.size === 'l'}<span class="disc" class:bad={f.tone === 'bad'} class:ok={f.tone === 'ok'}><Icon name="clock" size={16} /></span>{/if}
    <span class="txt grow">
      <span class="head">
        {#if m.size !== 'l'}{@render logo(13)}{/if}
        <span class="lbl">{m.size === 'l' ? `${name} · ${short(m.window)}` : name}</span>
      </span>
      <span class="sub strong" class:bad={f.tone === 'bad'} title={f.at ?? f.text}>{m.size === 'l' ? f.text : `${short(m.window)}: ${f.text}`}</span>
      {#if m.size === 'l' && s.value !== null}{@render bar(s, m.provider)}{/if}
    </span>
  </div>
{:else if m.kind === 'usage'}
  {@const period = t(`widget.period.${m.period}`)}
  {@const label = oc ? toolLabel('opencode') : period}
  <div class="mod usage {m.size}" role="group" aria-label={oc ? `${label} · ${period}` : period}>
    {#if !usage}
      <span class="txt"><span class="head">{#if oc}{@render logo(13)}{/if}<span class="lbl">{label}</span></span><span class="sub">{oc && !data?.opencode ? t('panel.why.off') : t('panel.why.none')}</span></span>
    {:else if m.size === 's'}
      <span class="stack">
        <b class="num">{fmtCompact(usage.tokens)}</b>
        <span class="tiny head">{#if oc}{@render logo(10)}{/if}{t(`panel.period.${m.period}`)}</span>
      </span>
    {:else if m.size === 'w'}
      <span class="txt grow">
        <span class="head">{#if oc}{@render logo(13)}{/if}<span class="lbl">{label}</span></span>
        <span class="row"><b class="num val">{fmtCompact(usage.tokens)}</b><span class="unit">{t('panel.tokens')}</span></span>
        <span class="sub" title={t('panel.cost')}>{oc ? `${t(`panel.period.${m.period}`)} · ` : ''}{fmtMoney(usage.cost_usd, { compact: true })}</span>
      </span>
    {:else}
      {#if oc}
        <span class="txt"><span class="head">{@render logo(14)}<span class="lbl">{label}</span></span><span class="sub">{period}</span></span>
      {:else}
        <span class="head"><span class="lbl">{period}</span></span>
      {/if}
      <span class="stats">
        <span><span class="tiny dim">{t('panel.tokens')}</span><b class="num">{fmtCompact(usage.tokens)}</b></span>
        <span title={t('panel.cost')}><span class="tiny dim">{t('panel.costShort')}</span><b class="num">{fmtMoney(usage.cost_usd, { compact: true })}</b></span>
        <span><span class="tiny dim">{t('panel.sessionsLabel')}</span><b class="num">{usage.sessions}</b></span>
        <span><span class="tiny dim">{t('panel.cache')}</span><b class="num">{hit === null ? '—' : fmtPct(hit)}</b></span>
      </span>
    {/if}
  </div>
{/if}

<style>
  .mod {
    position: relative;
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 100%;
    min-width: 0;
    padding: 9px 11px;
    border: 0.5px solid var(--cc-edge);
    border-radius: 16px;
    background: var(--cc-tile);
    color: var(--ink);
    font: inherit;
    text-align: left;
    overflow: hidden;
  }
  button.mod {
    cursor: default;
    transition: background var(--dur) var(--ease), transform 120ms var(--ease);
  }
  button.mod:hover {
    background: var(--cc-tile-hover);
  }
  button.mod:active {
    transform: scale(0.97);
  }
  button.mod:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .mod.s {
    flex-direction: column;
    justify-content: center;
    gap: 4px;
    padding: 6px;
  }
  .mod.m {
    flex-direction: column;
    align-items: stretch;
    justify-content: space-between;
    gap: 4px;
    padding: 10px 11px;
  }

  .txt {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .row,
  .head,
  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .row,
  .foot {
    justify-content: space-between;
  }
  .row.start {
    justify-content: flex-start;
    align-items: baseline;
    gap: 4px;
  }
  /* nothing in a tile may grow past it: one line, cut with an ellipsis */
  .row > *,
  .foot > *,
  .head > *,
  .stats > span > * {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row > .num,
  .foot > .sub:last-child {
    flex: none;
  }
  .push {
    margin-left: auto;
  }
  .lbl {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: -0.005em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 11px;
    color: var(--ink-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub.strong {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--ink);
    white-space: normal;
    line-height: 1.25;
  }
  .bad {
    color: var(--bad-ink) !important;
  }
  .tiny {
    font-size: 10px;
    color: var(--ink-2);
    white-space: nowrap;
  }
  .dim {
    color: var(--ink-3);
  }
  .num {
    font-variant-numeric: tabular-nums;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .val {
    font-size: 14px;
  }
  .small {
    font-size: 11px;
  }
  .mid {
    font-size: 16px;
  }
  .hero {
    font-size: 26px;
    line-height: 1;
  }
  .unit {
    margin-right: auto;
    font-size: 11px;
    font-weight: 500;
    color: var(--ink-2);
  }
  .logo {
    display: inline-grid;
    place-items: center;
    color: var(--ink-2);
  }

  .disc {
    display: grid;
    place-items: center;
    flex: none;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--cc-track);
    color: var(--ink);
    transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
  }
  .disc.bad {
    background: color-mix(in srgb, var(--critical) 18%, transparent);
    color: var(--bad-ink);
  }
  .disc.ok {
    background: color-mix(in srgb, var(--good) 18%, transparent);
    color: var(--good-ink);
  }
  /* round toggles sit inside their cell */
  .action.s {
    width: 52px;
    height: 52px;
    margin: 6px;
    padding: 0;
    border-radius: 50%;
    justify-content: center;
  }
  .action.s .disc {
    width: 100%;
    height: 100%;
    background: transparent;
  }
  .action.on,
  .action.on:hover {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: transparent;
  }
  .mod.done .disc {
    background: color-mix(in srgb, var(--good) 22%, transparent);
    color: var(--good-ink);
  }
  button.mod:disabled {
    cursor: progress;
  }
  .action.on .disc {
    color: var(--accent-ink);
  }
  .limit.w {
    gap: 8px;
    padding: 9px 10px;
  }
  .usage.w {
    padding: 0 13px;
    line-height: 1.15;
  }
  .limit.w .lbl,
  .forecast.w .lbl {
    font-size: 11.5px;
  }
  .action.w {
    gap: 7px;
    padding: 9px 10px;
  }
  .action.w .disc {
    width: 26px;
    height: 26px;
  }
  .action.w .lbl {
    font-size: 11.5px;
  }
  .action.w.on .disc {
    background: color-mix(in srgb, var(--accent-ink) 20%, transparent);
  }
  .spin .disc :global(svg) {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .track {
    display: block;
    width: 100%;
    height: 6px;
    border-radius: 3px;
    background: var(--cc-track);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: 3px;
    transition: width 600ms var(--ease);
  }
  .vfill {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    opacity: 0.9;
    transition: height 600ms var(--ease);
  }
  .stack {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .limit.s.bar .stack .tiny {
    color: inherit;
  }
  .center {
    display: grid;
    place-items: center;
    flex: 1;
  }
  .center .tiny {
    margin-top: 3px;
  }
  .big {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1;
    justify-content: center;
  }
  .pair {
    display: flex;
    justify-content: space-around;
    flex: 1;
    align-items: center;
  }
  .cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    min-width: 0;
    max-width: 50%;
  }
  .fit {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bars {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }
  .provider.w,
  .provider.l {
    flex-direction: column;
    align-items: stretch;
    justify-content: center;
    gap: 7px;
  }
  .provider.l .bars {
    flex-direction: row;
    gap: 14px;
  }
  .brow {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 7px;
    flex: 1;
    min-width: 0;
  }
  .brow:has(.when) {
    grid-template-columns: auto 1fr auto;
    grid-template-rows: auto auto;
  }
  .brow .when {
    grid-column: 1 / -1;
    font-size: 10px;
  }
  .brow .key {
    min-width: 30px;
  }
  .brow .val {
    font-size: 11px;
    color: var(--ink);
    min-width: 30px;
    text-align: right;
  }
  .stats {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 10px;
  }
  .stats > span {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .stats {
    gap: 6px 8px;
  }
  .stats .num {
    font-size: 13px;
  }
  @media (prefers-reduced-motion: reduce) {
    .fill,
    .vfill,
    button.mod,
    .disc {
      transition: none;
    }
    .spin .disc :global(svg) {
      animation: none;
    }
  }
</style>
