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
  import { api, type LiveCache, type Settings } from '../../lib/api'
  import { fmtClock, fmtCompact, fmtInt, fmtMoney, fmtPct, fmtSpan, fmtTime, limitName, limitShown, t, toolLabel, windowLabel, type LimitMode } from '../../lib/i18n.svelte'
  import { nameAccount } from '../../lib/store.svelte'
  import { page } from '../../lib/page.svelte'
  import BrandIcon from '../BrandIcon.svelte'
  import Icon from '../Icon.svelte'
  import Ring from './Ring.svelte'
  import CachePie from '../CachePie.svelte'
  import { cacheClient, cacheName, cacheTone, mmss, refreshError } from '../../lib/caches'

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
  // another account's tile names it, and then the provider's account in use is named too
  const account = $derived.by(() => {
    if (m.provider !== 'openai' && m.provider !== 'anthropic') return null
    if (m.account) return m.account
    return settings.panel.modules.some((x) => x.provider === m.provider && x.account) ? (data?.limits.find((l) => l.provider === m.provider && l.active)?.account ?? null) : null
  })
  // Antigravity's pools and other accounts by their short names, as on the widget; the logo stands beside them
  const name = $derived(
    m.provider === 'google'
      ? t(`widget.pool.google.${m.pool === 'gemini' ? 'gemini' : '3p'}`)
      : account
        ? nameAccount(account, m.provider as Provider, data?.accounts ?? [], settings.account_names, true)
        : limitName(m.provider, m.pool),
  )
  const short = (w: string) => t(`panel.window.${w}`)

  type Shown = { value: number | null; used: number | null; text: string; sub: string; why: string; behind: boolean; locked: boolean }
  function shown(l: PanelLimit | null): Shown {
    // its own percentage would read as room that a full week does not leave
    if (live(l) && l.blocked) return { value: 0, used: null, text: '', sub: t('panel.why.locked'), why: t('panel.short.locked'), behind: false, locked: true }
    if (live(l)) {
      const v = limitShown(l.used_pct, mode)
      return { value: v, used: l.used_pct, text: fmtPct(v), sub: t(`panel.${mode}`), why: '', behind: l.state === 'behind', locked: false }
    }
    const k = !l ? 'none' : l.state === 'reset' ? 'reset' : 'stale'
    return { value: null, used: null, text: '—', sub: t(`panel.why.${k}`), why: t(`panel.short.${k}`), behind: false, locked: false }
  }
  // a locked window says why where the others give their reset
  const note = (s: Shown, l: PanelLimit | null) => (s.locked ? s.why : resetLeft(l) ? `↻ ${resetLeft(l)}` : '')
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
    if (live(l) && l.blocked) return { text: t('limits.status.locked'), short: '', tone: 'bad' }
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

  const actionIcon: Record<string, string> = { open: 'app', widget: 'pip', rescan: 'sync', settings: 'gear' }
  const active = $derived(m.action === 'widget' ? !!data?.widget_visible : m.action === 'rescan' ? !!data?.scanning : false)
  const scanning = $derived(!!data?.scanning)
  const actionLabel = $derived(
    m.action === 'widget' ? t(data?.widget_visible ? 'panel.action.widget.on' : 'panel.action.widget.off') : m.action === 'rescan' && scanning ? t('panel.scan.running') : m.action === 'rescan' && scanned ? t('panel.scan.done') : t(`panel.action.${m.action}`),
  )
  const act = (a: string) => () => !edit && onaction?.(a)

  // the cache countdown ticks by the second, and only while there is one to show
  let clock = $state(Date.now())
  $effect(() => {
    if (m.kind !== 'caches' || !data?.caches.length || !page.visible) return
    clock = Date.now()
    const id = setInterval(() => (clock = Date.now()), 1000)
    return () => clearInterval(id)
  })
  const caches = $derived(m.kind === 'caches' ? (data?.caches ?? []).filter((c) => c.expires_ms > clock) : [])
  // until its refresh reaches the logs, a row's button keeps turning
  let asked = $state<string[]>([])
  // why a refresh did not go, in place of the row's second line for a few seconds
  let failed = $state<Record<string, string>>({})
  function refreshRow(c: LiveCache) {
    if (edit) return
    // the hook is added from the app, which asks first
    if (!data?.cache_refresh) return onaction?.('cache')
    if (!c.refreshable) return
    asked = [...asked, c.session_id]
    api
      .refreshCache(c.session_id)
      .catch((e) => {
        failed = { ...failed, [c.session_id]: refreshError(e) }
        setTimeout(() => {
          const next = { ...failed }
          delete next[c.session_id]
          failed = next
        }, 6000)
      })
      .finally(() => setTimeout(() => (asked = asked.filter((id) => id !== c.session_id)), 1500))
  }
</script>

{#snippet logo(size: number)}
  <span class="logo"><BrandIcon provider={providerOf(m)} {size} always /></span>
{/snippet}

{#snippet val(s: Shown, size: number)}{#if s.locked}<Icon name="lock" {size} />{:else}{s.behind ? '~' : ''}{s.text}{/if}{/snippet}

{#snippet bar(s: Shown, p: string)}
  <span class="track"><span class="fill" style:width="{s.value ?? 0}%" style:background={color(s.used, p)}></span></span>
{/snippet}

{#if m.kind === 'action'}
  <button class="mod action {m.size}" class:on={active} class:spin={m.action === 'rescan' && scanning} class:done={m.action === 'rescan' && scanned && !scanning} title={actionLabel} aria-label={actionLabel} aria-pressed={m.action === 'widget' ? active : undefined} aria-busy={m.action === 'rescan' ? scanning : undefined} disabled={m.action === 'rescan' && scanning && !edit} tabindex={edit ? -1 : 0} onclick={act(m.action)}>
    <span class="disc"><Icon name={m.action === 'rescan' && scanned && !scanning ? 'check' : actionIcon[m.action]} size={m.size === 's' ? 22 : 16} /></span>
    {#if m.size === 'w'}<span class="lbl">{actionLabel}</span>{/if}
  </button>
{:else if m.kind === 'scan'}
  <button class="mod scan w" class:spin={scanning} class:done={scanned && !scanning} aria-busy={scanning} disabled={scanning && !edit} tabindex={edit ? -1 : 0} onclick={act('rescan')} title={t('panel.action.rescan')}>
    <span class="disc"><Icon name={scanned && !scanning ? 'check' : 'sync'} size={16} /></span>
    <span class="txt">
      <span class="lbl">{t('panel.scan')}</span>
      <span class="sub">{scanning ? t('panel.scan.running') : scanned ? t('panel.scan.done') : data?.last_scan_ms ? t('panel.scan.ago', { span: fmtSpan(Math.max(0, now - data.last_scan_ms)) }) : t('panel.scan.never')}</span>
    </span>
  </button>
{:else if m.kind === 'limit'}
  {@const s = shown(one)}
  <div class="mod limit {m.size} {m.style}" role="group" aria-label="{name} · {windowLabel(m.window)}: {s.locked ? s.sub : `${s.text} ${s.sub}`}">
    {#if m.size === 's'}
      {#if m.style === 'bar'}
        <span class="vfill" style:height="{s.value ?? 0}%" style:background={color(s.used, m.provider)}></span>
        <span class="stack">
          <b class="num">{@render val(s, 14)}</b>
          <span class="tiny">{short(m.window)}</span>
        </span>
      {:else}
        <Ring value={s.value} color={color(s.used, m.provider)} size={40} stroke={4}><b class="num small">{#if s.locked}<Icon name="lock" size={14} />{:else}{s.value === null ? '—' : Math.round(s.value)}{/if}</b></Ring>
        <span class="tiny">{short(m.window)}</span>
      {/if}
    {:else if m.size === 'w'}
      {#if m.style === 'bar'}
        <span class="txt grow">
          <span class="row"><span class="lbl">{name}</span><b class="num val">{@render val(s, 13)}</b></span>
          {@render bar(s, m.provider)}
          <span class="sub" title={resetAt(one) || s.sub}>{short(m.window)}{note(s, one) ? ` · ${note(s, one)}` : s.value === null ? ` · ${s.why}` : ''}</span>
        </span>
      {:else}
        <Ring value={s.value} color={color(s.used, m.provider)} size={38} stroke={4}>{@render logo(13)}</Ring>
        <span class="txt">
          <span class="lbl">{name}</span>
          <span class="row start"><b class="num val">{@render val(s, 15)}</b><span class="unit">{short(m.window)}</span></span>
          <span class="sub" title={resetAt(one) || s.sub}>{note(s, one) || (s.value === null ? s.why : ' ')}</span>
        </span>
      {/if}
    {:else}
      {@const f = forecast(one)}
      <span class="head">{@render logo(14)}<span class="lbl">{name}</span></span>
      {#if m.style === 'bar'}
        <span class="big">
          <b class="num hero">{@render val(s, 26)}</b>
          <span class="sub" title={s.sub}>{s.locked || s.value === null ? s.why : s.sub} · {short(m.window)}</span>
          {@render bar(s, m.provider)}
        </span>
      {:else}
        <span class="center">
          <Ring value={s.value} color={color(s.used, m.provider)} size={70} stroke={6.5}>
            <b class="num mid">{@render val(s, 20)}</b>
            <span class="tiny">{s.locked || s.value === null ? '' : s.sub}</span>
          </Ring>
        </span>
      {/if}
      <span class="foot">
        <span class="sub" title={resetAt(one) || (s.locked ? s.sub : undefined)}>{note(s, one) || short(m.window)}</span>
        {#if f.short}<span class="sub" class:bad={f.tone === 'bad'} title={f.at ?? f.text}>{f.short}</span>{/if}
      </span>
    {/if}
  </div>
{:else if m.kind === 'provider'}
  {@const rows = both.map((b) => ({ ...b, s: shown(b.l) }))}
  <div class="mod provider {m.size} {m.style}" role="group" aria-label="{name}: {rows.map((r) => `${windowLabel(r.w)} ${r.s.locked ? r.s.why : r.s.text}`).join(', ')}">
    {#if m.size === 'm' && m.style !== 'bar'}
      <span class="head">{@render logo(14)}<span class="lbl">{name}</span></span>
      <span class="pair">
        {#each rows as r (r.w)}
          <span class="cell">
            <Ring value={r.s.value} color={color(r.s.used, m.provider)} size={50} stroke={5}><b class="num small">{#if r.s.locked}<Icon name="lock" size={16} />{:else}{r.s.value === null ? '—' : fmtPct(r.s.value)}{/if}</b></Ring>
            <span class="tiny">{short(r.w)}</span>
            <span class="tiny dim fit" title={r.s.locked || r.s.value === null ? r.s.sub : resetAt(r.l) || undefined}>{note(r.s, r.l) || r.s.why || ' '}</span>
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
            <b class="num tiny val">{@render val(r.s, 11)}</b>
            {#if m.size !== 'w'}<span class="tiny dim when fit" title={resetAt(r.l) || (r.s.locked ? r.s.sub : undefined)}>{note(r.s, r.l) || r.s.why}</span>{/if}
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
{:else if m.kind === 'caches'}
  <div class="mod caches {m.size}" role="group" aria-label={t('panel.caches.title')}>
    {#if m.size === 't'}
      <span class="head top">
        <Icon name="clock" size={14} />
        <span class="lbl">{t('panel.caches.title')}</span>
        {#if caches.length}<button class="more push" tabindex={edit ? -1 : 0} onclick={act('cache')}>{t('panel.caches.count', { n: caches.length })}<Icon name="chevron" size={10} /></button>{/if}
      </span>
    {/if}
    {#if !settings.cache_timer || !caches.length}
      <span class="sub none">{settings.cache_timer ? t('panel.caches.none') : t('panel.caches.off')}</span>
    {:else}
      {#each caches.slice(0, m.size === 't' ? 3 : 1) as c (c.session_id)}
        {@const left = c.expires_ms - clock}
        {@const tone = cacheTone(c, left)}
        {@const busy = c.refreshing || asked.includes(c.session_id)}
        {@const why = failed[c.session_id] ?? (c.refresh_failed ? t('cache.live.failed') : null)}
        <span class="crow">
          <CachePie frac={left / c.ttl_ms} {tone} size={16} />
          <span class="txt grow"><span class="lbl">{cacheName(c)}</span><span class="sub" class:bad={!!why}>{why ?? c.branch ?? cacheClient(c.client)}</span></span>
          <b class="num ctime {tone}">{mmss(left)}</b>
          <button
            class="rf"
            class:busy
            tabindex={edit ? -1 : 0}
            aria-label={t('panel.caches.refresh', { name: cacheName(c) })}
            title={!data?.cache_refresh ? t('panel.caches.setup') : c.refreshable || busy ? t('cache.live.refresh.help') : t('cache.live.notWaiting')}
            disabled={!!data?.cache_refresh && !c.refreshable && !busy}
            aria-busy={busy}
            onclick={() => refreshRow(c)}><Icon name={busy ? 'sync' : 'refresh'} size={13} /></button
          >
        </span>
      {/each}
      {#if m.size === 'l' && caches.length > 1}<span class="tiny dim plus">+{caches.length - 1}</span>{/if}
    {/if}
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
        <span><span class="tiny dim">{t('panel.sessionsLabel')}</span><b class="num">{fmtInt(usage.sessions)}</b></span>
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
  .mod.caches.t {
    flex-direction: column;
    align-items: stretch;
    justify-content: flex-start;
    gap: 0;
    padding: 10px 11px 6px;
  }
  .caches .top {
    margin-bottom: 4px;
  }
  .crow {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
    height: 52px;
  }
  .caches.l .crow {
    flex: 1;
  }
  .caches .none {
    margin: auto;
    text-align: center;
  }
  .ctime {
    flex: none;
    font-size: 13px;
  }
  .ctime.soon {
    color: color-mix(in srgb, var(--warning) 72%, var(--ink));
  }
  .ctime.urgent {
    color: color-mix(in srgb, var(--critical) 82%, var(--ink));
  }
  .more {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--ink-3);
    font: inherit;
    font-size: 11px;
  }
  .more:hover {
    color: var(--ink);
  }
  .plus {
    flex: none;
  }
  .crow .sub.bad {
    color: var(--bad-ink);
  }
  .rf {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: color-mix(in srgb, var(--ink) 9%, transparent);
    color: var(--ink);
  }
  .rf:hover:not(:disabled) {
    background: color-mix(in srgb, var(--ink) 15%, transparent);
  }
  .rf:disabled {
    opacity: 0.4;
  }
  .rf.busy {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .rf.busy :global(svg) {
    animation: rf-spin 0.9s linear infinite;
  }
  .rf:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  @keyframes rf-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rf.busy :global(svg) {
      animation: none;
    }
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
  /* small buttons fill their cell like the other tiles */
  .action.s {
    padding: 0;
  }
  .action.s .disc {
    width: 100%;
    height: 100%;
    border-radius: inherit;
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
  /* three lines beside the ring must fit the one-row tile */
  .limit.w.ring .txt {
    line-height: 1.2;
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
  .limit.s.ring {
    gap: 2px;
    padding: 4px 6px;
  }
  .limit.s.ring .tiny {
    line-height: 12px;
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
  /* a title and two bars in a one-row tile: tight lines, or they spill past its top and bottom */
  .provider.w,
  .provider.l {
    flex-direction: column;
    align-items: stretch;
    justify-content: center;
    gap: 4px;
    padding-block: 8px;
  }
  .provider.w .head,
  .provider.l .head {
    line-height: 15px;
  }
  .provider.w .bars {
    gap: 3px;
  }
  .provider.l .bars {
    flex-direction: row;
    gap: 14px;
  }
  .provider.w .brow,
  .provider.l .brow {
    row-gap: 1px;
    line-height: 13px;
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
