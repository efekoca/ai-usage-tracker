<script lang="ts">
  import { onMount } from 'svelte'
  import { api, on, type LiveCache } from '../lib/api'
  import { app } from '../lib/store.svelte'
  import { clockCase, fmtCompact, fmtMoney, t } from '../lib/i18n.svelte'
  import { cacheClient, cacheName, cacheTone, mmss, shortModel } from '../lib/caches'
  import { ensureRefresh, loadRefreshInfo, refresh } from '../lib/refresh.svelte'
  import CachePie from './CachePie.svelte'
  import Icon from './Icon.svelte'

  let caches = $state<LiveCache[]>([])
  let now = $state(Date.now())
  let open = $state<string | null>(null)
  let menu = $state<string | null>(null)

  let gen = 0
  async function load() {
    const mine = ++gen
    try {
      const [list] = await Promise.all([api.liveCaches(), loadRefreshInfo()])
      if (mine === gen) caches = list
    } catch {
      /* backend busy; the next refresh retries */
    }
  }

  onMount(() => {
    load()
    const subs = [on('data-changed', load)]
    // the countdown ticks by the second; a new request reaches the list with the next scan
    const tick = setInterval(() => (now = Date.now()), 1000)
    const poll = setInterval(load, 20_000)
    const away = (e: PointerEvent) => {
      if (!(e.target as Element | null)?.closest?.('.menu-wrap')) menu = null
    }
    window.addEventListener('pointerdown', away)
    return () => {
      clearInterval(tick)
      clearInterval(poll)
      window.removeEventListener('pointerdown', away)
      subs.forEach((p) => p.then((u) => u()))
    }
  })

  const live = $derived(caches.filter((c) => c.expires_ms > now))
  // while a refresh is on its way, the list looks again sooner than its usual round
  $effect(() => {
    if (!caches.some((c) => c.refreshing)) return
    const id = setInterval(load, 3000)
    return () => clearInterval(id)
  })

  // a row waits for its refresh to reach the logs, then shows the cache's new time
  let asked = $state<string[]>([])
  async function refreshNow(c: LiveCache) {
    if (!refresh.info?.installed) {
      if (!(await ensureRefresh())) return
      // the row as it is once the hook is there: a conversation waiting already can go at once
      await load()
      c = caches.find((x) => x.session_id === c.session_id) ?? c
    }
    if (!c.refreshable) return
    asked = [...asked, c.session_id]
    try {
      await api.refreshCache(c.session_id)
    } catch {
      /* it ran out or went on meanwhile; the reload shows how it is now */
    }
    asked = asked.filter((id) => id !== c.session_id)
    load()
  }
  async function flipAuto(c: LiveCache) {
    menu = null
    if (!c.auto && !(await ensureRefresh())) return
    await api.setCacheAuto(c.session_id, !c.auto).catch(() => {})
    load()
  }
  // when an automatic refresh comes: the lead set in Settings, a 5-minute cache in its last minute
  const autoAt = (c: LiveCache) => c.expires_ms - (c.ttl_ms <= 300_000 ? 1 : (app.settings?.cache_lead ?? 2)) * 60_000
  function status(c: LiveCache): { text: string; tone: string; auto: boolean } {
    if (c.refreshing || asked.includes(c.session_id)) return { text: t('cache.live.refreshing'), tone: 'busy', auto: false }
    if (c.auto && c.auto_capped) return { text: t('cache.live.autoStopped'), tone: '', auto: true }
    if (c.auto && c.refreshable) return { text: t('cache.live.refreshAt', { at: clockCase(autoAt(c), 'at') }), tone: '', auto: true }
    return { text: t('cache.live.endsAt', { at: clockCase(c.expires_ms, 'at') }), tone: '', auto: false }
  }

  function untrack(c: LiveCache) {
    menu = null
    caches = caches.filter((x) => x.session_id !== c.session_id)
    api.dropCache(c.session_id).catch(load)
  }

  const meta = (c: LiveCache) =>
    [shortModel(c.model), cacheClient(c.client), t(c.ttl_ms <= 300_000 ? 'cache.live.ttl.5' : 'cache.live.ttl.60'), c.branch ? '' : t('cache.live.since', { at: clockCase(c.started_ms, 'from') })].filter(Boolean).join(' · ')
  const money = (usd: number | null) => (usd === null ? '—' : fmtMoney(usd))
</script>

{#if app.settings?.cache_timer}
  <section class="card live" aria-labelledby="live-caches">
    <div class="head">
      <h2 id="live-caches">{t('cache.live.title')}</h2>
      <span class="grow"></span>
      {#if refresh.info?.today}
        <span class="today" title={t('cache.live.today.help')}><Icon name="refresh" size={13} />{t('cache.live.today', { n: refresh.info.today, cost: fmtMoney(refresh.info.today_usd) })}</span>
      {/if}
      <button class="btn ghost square" aria-label={t('cache.live.settings')} title={t('cache.live.settings')} onclick={() => (app.view = 'settings')}><Icon name="gear" size={15} /></button>
    </div>
    {#if !live.length}
      <p class="empty">{t('cache.live.empty')}</p>
    {:else}
      <ul>
        {#each live as c (c.session_id)}
          {@const left = c.expires_ms - now}
          {@const tone = cacheTone(c, left)}
          {@const name = cacheName(c)}
          {@const st = status(c)}
          <li>
            <div class="row">
              <button class="who" aria-expanded={open === c.session_id} aria-label={t('cache.live.details', { name })} onclick={() => (open = open === c.session_id ? null : c.session_id)}>
                <span class="lead" title={t('cache.live.left', { t: mmss(left) })}>
                  <CachePie frac={left / c.ttl_ms} {tone} />
                  <b class="time {tone}">{mmss(left)}</b>
                </span>
                <span class="txt">
                  <span class="line">
                    <span class="name" class:hidden={!c.project}>{name}</span>
                    {#if c.branch}<span class="chip"><Icon name="branch" size={11} />{c.branch}</span>{/if}
                    <span class="chev" class:open={open === c.session_id}><Icon name="chevron" size={12} /></span>
                  </span>
                  <span class="meta">{meta(c)}</span>
                </span>
              </button>
              <span class="status {st.tone}" class:urgent={tone === 'urgent' && !st.auto && st.tone !== 'busy'} title={st.auto && c.auto_capped ? t('cache.live.autoStopped.help') : undefined}>
                {#if st.auto}<Icon name="refresh" size={12} />{/if}{st.text}
              </span>
              <button
                class="btn go"
                disabled={c.refreshing || asked.includes(c.session_id) || (!!refresh.info?.installed && !c.refreshable)}
                title={refresh.info?.installed && !c.refreshable && !c.refreshing ? t('cache.live.notWaiting') : t('cache.live.refresh.help')}
                onclick={() => refreshNow(c)}>{t('cache.live.refresh')}</button
              >
              <div class="menu-wrap">
                <button class="btn ghost square" aria-label={t('cache.live.more', { name })} aria-haspopup="menu" aria-expanded={menu === c.session_id} onclick={() => (menu = menu === c.session_id ? null : c.session_id)}><Icon name="more" size={16} /></button>
                {#if menu === c.session_id}
                  <div class="menu" role="menu" tabindex="-1" onkeydown={(e) => e.key === 'Escape' && (menu = null)}>
                    <button role="menuitemcheckbox" aria-checked={c.auto} onclick={() => flipAuto(c)}><span class="check" class:on={c.auto}><Icon name="check" size={13} /></span>{t('cache.live.auto')}</button>
                    <button role="menuitem" onclick={() => untrack(c)}><span class="check"></span>{t('cache.live.untrack')}</button>
                  </div>
                {/if}
              </div>
            </div>
            {#if open === c.session_id}
              <dl class="facts">
                <div><dt>{t('cache.live.tokens')}</dt><dd>{t('cache.live.tokensValue', { v: fmtCompact(c.tokens) })}</dd></div>
                <div><dt title={t('cache.live.refreshCost.help')}>{t('cache.live.refreshCost')}</dt><dd>{money(c.refresh_usd)}</dd></div>
                <div><dt title={t('cache.live.rewriteCost.help')}>{t('cache.live.rewriteCost')}</dt><dd>{money(c.rewrite_usd)}</dd></div>
                <div><dt>{t('cache.live.here')}</dt><dd>{c.refreshes ? t('cache.live.hereValue', { n: c.refreshes, cost: fmtMoney(c.refreshes_usd) }) : t('cache.live.hereNone')}</dd></div>
              </dl>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .live {
    margin-bottom: 16px;
    padding-bottom: 6px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }
  .grow {
    flex: 1;
  }
  .square {
    width: 30px;
    padding: 0;
    justify-content: center;
    color: var(--ink-3);
  }
  .empty {
    padding: 4px 0 14px;
    font-size: 13px;
    color: var(--ink-3);
  }
  ul {
    list-style: none;
    margin: 0 -20px;
    padding: 0;
  }
  li {
    border-top: 0.5px solid var(--hairline);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 20px 8px 12px;
  }
  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 6px 8px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
  }
  .who:hover {
    background: var(--surface-hover);
  }
  .who:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .lead {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 78px;
  }
  .time {
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }
  .time.soon {
    color: color-mix(in srgb, var(--warning) 72%, var(--ink));
  }
  .time.urgent {
    color: color-mix(in srgb, var(--critical) 82%, var(--ink));
  }
  .txt {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .name {
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name.hidden {
    color: var(--ink-2);
    font-weight: 500;
  }
  .chip {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 7px;
    border-radius: 6px;
    background: var(--surface-2);
    font-size: 11.5px;
    color: var(--ink-2);
    white-space: nowrap;
  }
  .chev {
    flex: none;
    display: inline-flex;
    color: var(--ink-3);
    transition: transform var(--dur) var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .meta {
    font-size: 12px;
    color: var(--ink-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status {
    flex: none;
    font-size: 12.5px;
    color: var(--ink-3);
    font-variant-numeric: tabular-nums;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .status.urgent {
    color: var(--bad-ink);
  }
  .status.busy {
    color: var(--accent);
  }
  .today {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--ink-3);
    font-variant-numeric: tabular-nums;
  }
  .go {
    flex: none;
    border-color: transparent;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    font-weight: 600;
  }
  .go:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .check {
    display: inline-flex;
    width: 13px;
    margin-right: 9px;
    color: transparent;
  }
  .check.on {
    color: var(--accent);
  }
  .menu-wrap {
    position: relative;
    flex: none;
  }
  .menu {
    position: absolute;
    right: 0;
    top: 34px;
    z-index: 5;
    min-width: 180px;
    padding: 5px;
    border-radius: 10px;
    background: var(--surface);
    border: 0.5px solid var(--hairline-strong);
    box-shadow: var(--shadow-pop);
  }
  .menu button {
    display: flex;
    align-items: center;
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--ink);
    font: inherit;
    font-size: 13px;
    text-align: left;
  }
  .menu button:hover,
  .menu button:focus-visible {
    background: var(--surface-hover);
    outline: none;
  }
  .facts {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    margin: 0 20px 12px 112px;
    padding: 12px 16px;
    border-radius: 10px;
    background: var(--surface-2);
    border: 0.5px solid var(--hairline);
  }
  .facts > div {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .facts > div + div {
    padding-left: 16px;
    border-left: 0.5px solid var(--hairline);
  }
  .facts > div:not(:last-child) {
    padding-right: 12px;
  }
  dt {
    font-size: 11.5px;
    color: var(--ink-3);
  }
  dd {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  @container main (max-width: 760px) {
    .status {
      display: none;
    }
    .facts {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      row-gap: 10px;
      margin-left: 20px;
    }
    .facts > div:nth-child(3) {
      padding-left: 0;
      border-left: 0;
    }
  }
</style>
