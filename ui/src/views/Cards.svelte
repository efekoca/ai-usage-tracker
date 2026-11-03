<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog'
  import { app, latest, saveSettings } from '../lib/store.svelte'
  import { api, providersOf, type CardFormat, type CardInfo, type CardSettings, type CardTheme, type Period, type PlansFile } from '../lib/api'
  import { saveAs } from '../lib/dialogs'
  import { fmtDate, fmtInt, i18n, localDate, t } from '../lib/i18n.svelte'
  import { planMultiple } from '../lib/plans'
  import { buildCalendar, buildHours, buildSummary, buildWrapped, designsFor, periodBounds, previousPeriod, weekTitle, type Card, type CardKind, type Design, type Facts } from '../lib/cards'
  import { ACCENTS, drawCard, sizeOf, THEME_ACCENT, themeSwatch, type DrawOptions } from '../lib/cardCanvas'
  import Segmented from '../components/Segmented.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Icon from '../components/Icon.svelte'

  const KINDS: CardKind[] = ['day', 'week', 'month', 'year']
  const FORMATS: CardFormat[] = ['story', 'square', 'landscape', 'wallpaper']
  const THEMES: CardTheme[] = ['navy', 'light', 'sunset', 'aurora']
  const INFO: CardInfo[] = ['cost', 'plan', 'change', 'tools', 'models', 'badges', 'arc']
  const ACCENT_NAMES = ['blue', 'violet', 'mint', 'amber', 'orange', 'pink']
  const DESIGN_ICON: Record<Design, string> = { summary: 'chart', calendar: 'daily', hours: 'clock', wrapped: 'report' }
  const DEFAULT_PREFS: CardSettings = { theme: 'navy', accent: 'theme', format: 'story', hide: [] }

  let kind = $state<CardKind>('month')
  let design = $state<Design>('summary')
  /** Any day inside the chosen period. */
  let at = $state(new Date())
  /** `undefined` while loading, `null` for a period without use. */
  let facts = $state.raw<Facts | null | undefined>(undefined)
  let failed = $state('')
  let busy = $state<'save' | 'copy' | 'all' | null>(null)
  let status = $state<{ ok: boolean; text: string; open?: boolean } | null>(null)
  let slide = $state(0)
  let preview: HTMLCanvasElement | undefined = $state()
  const thumbs: HTMLCanvasElement[] = $state([])
  let plans: PlansFile | null = null
  /** The period on screen, so a data refresh keeps the card and the saved note in place. */
  let shown = ''
  /** The choice just made, shown until the saved settings come back with it. */
  let chosen = $state<CardSettings | null>(null)

  const prefs = $derived(chosen ?? app.settings?.cards ?? DEFAULT_PREFS)
  const designs = $derived(designsFor(kind))
  const active = $derived(designs.includes(design) ? design : 'summary')
  const format = $derived<CardFormat>(active === 'wrapped' ? 'story' : prefs.format)
  /** The sizes this card can take; a year story is only a story. */
  const formats = $derived<CardFormat[]>(active === 'wrapped' ? ['story'] : FORMATS)
  const screen = { w: Math.round(window.screen.width * devicePixelRatio), h: Math.round(window.screen.height * devicePixelRatio) }
  const size = $derived(sizeOf(format, screen))

  const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n)
  const moved = (k: CardKind, d: Date, n: number) =>
    k === 'day' ? addDays(d, n) : k === 'week' ? addDays(d, 7 * n) : k === 'month' ? new Date(d.getFullYear(), d.getMonth() + n, 1) : new Date(d.getFullYear() + n, 0, 1)
  const range = $derived(periodBounds(kind, at))
  const canNext = $derived(periodBounds(kind, moved(kind, at, 1)).from <= new Date())
  const label = $derived(
    kind === 'day' ? fmtDate(at, 'long') : kind === 'week' ? weekTitle(range.from, range.to, true) : kind === 'month' ? fmtDate(at, 'monthLong') : String(at.getFullYear()),
  )
  const stem = $derived(kind === 'year' ? String(range.from.getFullYear()) : kind === 'month' ? localDate(range.from).slice(0, 7) : localDate(range.from) + (kind === 'week' ? '_week' : ''))

  $effect(() => {
    void app.tick
    const k = kind
    const { from, to } = range
    const period: Period = { kind: 'custom', from: localDate(from), to: localDate(to) }
    const before = previousPeriod(k, from)
    const previous: Period = { kind: 'custom', from: localDate(before.from), to: localDate(before.to) }
    if (shown !== `${k}:${period.from}`) {
      shown = `${k}:${period.from}`
      facts = undefined
      status = null
      slide = 0
    }
    return latest(
      async (): Promise<Facts> => {
        // without the earlier period the card simply leaves the change out
        const earlier = api.earlierTotals(period, previous).catch(() => null)
        if (k === 'day') {
          const [report, day, prior, e] = await Promise.all([
            api.report(period),
            api.dayDetail(period.from),
            api.report({ kind: 'custom', from: localDate(addDays(from, -30)), to: localDate(addDays(from, -1)) }),
            earlier,
          ])
          return { kind: k, from, to, now: new Date(), report, day, prior, sessions: day.sessions, begun: 1, multiple: null, earlier: e, hide: [] }
        }
        const [report, value, p, sessions, e] = await Promise.all([api.report(period), api.planValue(period), plans ?? api.plans(), k === 'year' ? null : api.sessions(period), earlier])
        plans = p
        const multiple = planMultiple({ plans: p, settings: app.settings, accounts: app.accounts, value, fromFirstSeen: true }, providersOf(app.settings?.enabled_sources ?? []))
        return { kind: k, from, to, now: new Date(), report, day: null, prior: null, sessions: sessions?.sessions.length ?? null, begun: value.dates.length, multiple, earlier: e, hide: [] }
      },
      (f) => {
        facts = f.report.totals.events === 0 ? null : f
        failed = ''
      },
      (e) => {
        facts = null
        failed = String(e)
      },
    )
  })

  /** The card, or a year's slides; `undefined` while loading and `null` without use. */
  const built = $derived.by((): Card[] | null | undefined => {
    if (facts === undefined || facts === null) return facts
    void i18n.lang
    void i18n.currency
    const f = { ...facts, hide: [...prefs.hide] }
    const one = active === 'wrapped' ? null : active === 'calendar' ? buildCalendar(f) : active === 'hours' ? buildHours(f) : buildSummary(f)
    const cards = active === 'wrapped' ? buildWrapped(f) : one ? [one] : []
    if (!cards.length) return null
    return prefs.hide.includes('arc') ? cards.map((c) => ({ ...c, share: null })) : cards
  })
  const current = $derived(built ? built[Math.min(slide, built.length - 1)] : null)

  const options = (f: CardFormat): DrawOptions => ({
    format: f,
    theme: prefs.theme,
    accent: prefs.accent === 'theme' ? null : prefs.accent,
    family: getComputedStyle(document.body).fontFamily,
    screen,
  })

  $effect(() => {
    if (current && preview) drawCard(preview, current, options(format))
  })

  // thumbnails are drawn full size once, then copied down so five slides do not hold five full canvases
  $effect(() => {
    if (active !== 'wrapped' || !built) return
    const full = document.createElement('canvas')
    built.forEach((c, i) => {
      const thumb = thumbs[i]
      if (!thumb) return
      drawCard(full, c, options('story'))
      thumb.width = 216
      thumb.height = 384
      const ctx = thumb.getContext('2d')
      ctx?.drawImage(full, 0, 0, thumb.width, thumb.height)
    })
  })

  function pick(k: CardKind) {
    kind = k
    at = new Date()
  }

  function setPrefs(patch: Partial<CardSettings>) {
    const next = { ...prefs, ...patch }
    chosen = next
    status = null
    saveSettings((s) => ({ cards: { ...s.cards, ...next } })).catch((e) => (status = { ok: false, text: t('common.error', { e: String(e) }) }))
  }

  function toggleInfo(x: CardInfo, on: boolean) {
    setPrefs({ hide: on ? prefs.hide.filter((h) => h !== x) : [...prefs.hide, x] })
  }

  /** Arrow keys move the choice inside a group of tiles, as in a segmented control. */
  function arrows<T>(e: KeyboardEvent, list: readonly T[], value: T, set: (v: T) => void) {
    const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0
    if (!step) return
    e.preventDefault()
    const next = list[(list.indexOf(value) + step + list.length) % list.length]
    if (next !== value) set(next)
    const group = e.currentTarget as HTMLElement
    queueMicrotask(() => (group.querySelector('[aria-checked="true"]') as HTMLElement | null)?.focus())
  }

  const fileName = (f: CardFormat, i = 0) =>
    `AI-Usage_${stem}${active === 'summary' ? '' : `_${active}`}${active === 'wrapped' ? `_${i + 1}` : f === 'story' ? '' : `_${f}`}.png`

  async function png(card: Card, f: CardFormat): Promise<Uint8Array> {
    const canvas = document.createElement('canvas')
    drawCard(canvas, card, options(f))
    const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, 'image/png'))
    if (!blob) throw new Error('png')
    return new Uint8Array(await blob.arrayBuffer())
  }

  async function run(what: 'save' | 'copy' | 'all', task: () => Promise<{ text: string; open?: boolean } | null>) {
    status = null
    busy = what
    try {
      const done = await task()
      if (done) status = { ok: true, ...done }
    } catch (e) {
      status = { ok: false, text: t(what === 'copy' ? 'cards.copyFailed' : 'cards.failed', { e: String(e) }) }
    } finally {
      busy = null
    }
  }

  const save = () =>
    run('save', async () => {
      if (!current) return null
      const path = await saveAs(fileName(format, slide), 'PNG', 'png')
      if (!path) return null
      await api.saveImage(path, await png(current, format))
      return { text: t('cards.saved'), open: true }
    })

  const copy = () =>
    run('copy', async () => {
      if (!current) return null
      await api.copyImage(await png(current, format))
      return { text: t('cards.copied') }
    })

  const saveAll = () =>
    run('all', async () => {
      if (!built || !current) return null
      const dir = await open({ directory: true, title: t('cards.chooseFolder') })
      if (typeof dir !== 'string') return null
      const images =
        active === 'wrapped'
          ? await Promise.all(built.map(async (c, i) => ({ name: fileName('story', i), bytes: await png(c, 'story') })))
          : await Promise.all(FORMATS.map(async (f) => ({ name: fileName(f), bytes: await png(current, f) })))
      const written = await api.saveImages(dir, images)
      return { text: t('cards.savedAll', { n: written.length }), open: true }
    })
</script>

<header class="bar">
  <h1>{t('nav.cards')}</h1>
</header>
<p class="subtle small help">{t('cards.help')}</p>

<div class="controls">
  <Segmented label={t('cards.period')} value={kind} options={KINDS.map((k) => ({ value: k, label: t(`cards.kind.${k}`) }))} onchange={pick} />
  <div class="stepper">
    <button class="step" aria-label={t('cards.prev')} title={t('cards.prev')} onclick={() => (at = moved(kind, at, -1))}>
      <span class="back"><Icon name="chevron" size={16} /></span>
    </button>
    <span class="label num" aria-live="polite">{label}</span>
    <button class="step" aria-label={t('cards.next')} title={t('cards.next')} disabled={!canNext} onclick={() => (at = moved(kind, at, 1))}>
      <Icon name="chevron" size={16} />
    </button>
  </div>
</div>

<div class="studio">
  <section class="stage" aria-label={t('cards.preview')}>
    {#if failed}
      <p class="note muted small"><Icon name="warning" size={15} />{t('common.error', { e: failed })}</p>
    {:else if built === null}
      <p class="note muted small"><Icon name="info" size={15} />{t('cards.empty')}</p>
    {:else}
      <div class="shot {format}" class:loading={!current} style:aspect-ratio="{size.w} / {size.h}">
        <canvas bind:this={preview} class:hidden={!current}>{t('cards.aria', { title: label })}</canvas>
      </div>
      <p class="tiny subtle num caption">{t(`cards.format.${format}`)} · {fmtInt(size.w)} × {fmtInt(size.h)}</p>
      {#if active === 'wrapped' && built}
        <div class="rail" role="radiogroup" aria-label={t('cards.slides')} tabindex="-1" onkeydown={(e) => arrows(e, built.map((_, i) => i), slide, (i) => (slide = i))}>
          {#each built as c, i (i)}
            <button class="thumb" class:on={i === slide} role="radio" aria-checked={i === slide} aria-label="{c.pager}, {c.eyebrow}" tabindex={i === slide ? 0 : -1} onclick={() => (slide = i)}>
              <canvas bind:this={thumbs[i]}></canvas>
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  </section>

  <aside class="inspector">
    <div class="group">
      <span class="glabel" id="card-design">{t('cards.design')}</span>
      <div class="tiles two" role="radiogroup" aria-labelledby="card-design" tabindex="-1" onkeydown={(e) => arrows(e, designs, active, (d) => (design = d))}>
        {#each designs as d (d)}
          <button class="tile" role="radio" aria-checked={active === d} tabindex={active === d ? 0 : -1} onclick={() => (design = d)}>
            <Icon name={DESIGN_ICON[d]} size={16} />
            <span>{t(`cards.design.${d}`)}</span>
          </button>
        {/each}
      </div>
    </div>

    <div class="group">
      <span class="glabel" id="card-format">{t('cards.format')}</span>
      <div class="tiles two" role="radiogroup" aria-labelledby="card-format" tabindex="-1" onkeydown={(e) => arrows(e, formats, format, (f) => setPrefs({ format: f }))}>
        {#each FORMATS as f (f)}
          {@const s = sizeOf(f, screen)}
          <button class="tile format" role="radio" aria-checked={format === f} disabled={!formats.includes(f)} tabindex={format === f ? 0 : -1} onclick={() => setPrefs({ format: f })}>
            <span class="ratio" style:aspect-ratio="{s.w} / {s.h}" aria-hidden="true"></span>
            <span>{t(`cards.format.${f}`)}</span>
          </button>
        {/each}
      </div>
      {#if active === 'wrapped'}<p class="tiny subtle">{t('cards.format.wrappedNote')}</p>{/if}
    </div>

    <div class="group">
      <span class="glabel" id="card-theme">{t('cards.theme')}</span>
      <div class="themes" role="radiogroup" aria-labelledby="card-theme" tabindex="-1" onkeydown={(e) => arrows(e, THEMES, prefs.theme, (v) => setPrefs({ theme: v }))}>
        {#each THEMES as th (th)}
          {@const sw = themeSwatch(th)}
          <button class="theme" role="radio" aria-checked={prefs.theme === th} tabindex={prefs.theme === th ? 0 : -1} onclick={() => setPrefs({ theme: th })}>
            <span class="sample" style:background="linear-gradient(135deg, {sw.from}, {sw.to})" aria-hidden="true">
              <span class="dot" style:background={prefs.accent === 'theme' ? sw.accent : prefs.accent}></span>
            </span>
            <span class="tiny">{t(`cards.theme.${th}`)}</span>
          </button>
        {/each}
      </div>
    </div>

    <div class="group">
      <span class="glabel" id="card-accent">{t('cards.accent')}</span>
      <div class="accents" role="radiogroup" aria-labelledby="card-accent" tabindex="-1" onkeydown={(e) => arrows(e, ['theme', ...ACCENTS], prefs.accent, (v) => setPrefs({ accent: v }))}>
        <button
          class="accent auto"
          role="radio"
          aria-checked={prefs.accent === 'theme'}
          aria-label={t('cards.accent.theme')}
          title={t('cards.accent.theme')}
          tabindex={prefs.accent === 'theme' ? 0 : -1}
          style:--c={THEME_ACCENT[prefs.theme]}
          onclick={() => setPrefs({ accent: 'theme' })}
        ></button>
        {#each ACCENTS as a, i (a)}
          <button
            class="accent"
            role="radio"
            aria-checked={prefs.accent === a}
            aria-label={t(`cards.accent.${ACCENT_NAMES[i]}`)}
            title={t(`cards.accent.${ACCENT_NAMES[i]}`)}
            tabindex={prefs.accent === a ? 0 : -1}
            style:--c={a}
            onclick={() => setPrefs({ accent: a })}
          ></button>
        {/each}
      </div>
    </div>

    <div class="group">
      <span class="glabel">{t('cards.show')}</span>
      <ul class="info">
        {#each INFO as x (x)}
          <li>
            <span class="small">{t(`cards.info.${x}`)}</span>
            <Toggle checked={!prefs.hide.includes(x)} label={t(`cards.info.${x}`)} onchange={(v) => toggleInfo(x, v)} />
          </li>
        {/each}
      </ul>
    </div>

    <div class="actions">
      <button class="btn primary" onclick={save} disabled={!current || busy !== null}>
        {#if busy === 'save'}<span class="spin" aria-hidden="true"></span>{:else}<Icon name="download" size={15} />{/if}{t('cards.save')}
      </button>
      <button class="btn" onclick={copy} disabled={!current || busy !== null}>
        {#if busy === 'copy'}<span class="spin" aria-hidden="true"></span>{:else}<Icon name="copy" size={15} />{/if}{t('cards.copy')}
      </button>
      <button class="btn wide" onclick={saveAll} disabled={!current || busy !== null}>
        {#if busy === 'all'}<span class="spin" aria-hidden="true"></span>{:else}<Icon name="images" size={15} />{/if}{t(active === 'wrapped' ? 'cards.saveAll.slides' : 'cards.saveAll.formats')}
      </button>
    </div>
    {#if status}
      <p class="small state" class:bad={!status.ok} role={status.ok ? 'status' : 'alert'}>
        <Icon name={status.ok ? 'check' : 'warning'} size={14} /><span>{status.text}</span>
        {#if status.ok && status.open}<button class="btn ghost open" onclick={() => api.openLastImage()}>{t('cards.open')}</button>{/if}
      </p>
    {/if}
  </aside>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 0 4px;
  }
  .help {
    margin: 4px 0 18px;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    margin-bottom: 16px;
  }
  .stepper {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .step {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 8px;
    background: var(--surface-hover);
    color: var(--ink);
  }
  .step:disabled {
    background: transparent;
    opacity: 0.3;
  }
  .back {
    display: inline-flex;
    transform: rotate(180deg);
  }
  .label {
    min-width: 120px;
    padding: 0 6px;
    text-align: center;
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .studio {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 304px;
    gap: 16px;
    align-items: start;
  }
  .stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 18px;
    min-height: 560px;
    padding: 28px 24px;
    border-radius: 16px;
    background: var(--stage);
    box-shadow: inset 0 0 0 0.5px var(--hairline);
  }
  .note {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .shot {
    overflow: hidden;
    border-radius: 14px;
    box-shadow:
      0 0 0 0.5px var(--hairline-strong),
      var(--shadow-pop);
  }
  .shot.story {
    height: min(66vh, 640px);
  }
  .shot.square {
    width: min(100%, 520px);
  }
  .shot.landscape,
  .shot.wallpaper {
    width: min(100%, 760px);
  }
  .shot.loading {
    background: var(--surface-hover);
    box-shadow: inset 0 0 0 0.5px var(--hairline);
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  canvas.hidden {
    display: none;
  }
  .rail {
    display: flex;
    gap: 10px;
    max-width: 100%;
    overflow-x: auto;
    padding: 4px;
  }
  .thumb {
    flex: none;
    width: 54px;
    aspect-ratio: 9 / 16;
    padding: 0;
    overflow: hidden;
    border: 0;
    border-radius: 7px;
    background: var(--surface-hover);
    box-shadow: 0 0 0 0.5px var(--hairline-strong);
    opacity: 0.62;
    transition:
      opacity 160ms ease-out,
      box-shadow 160ms ease-out,
      transform 160ms ease-out;
  }
  .thumb.on {
    opacity: 1;
    box-shadow: 0 0 0 2px var(--accent);
  }

  .inspector {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 16px;
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow:
      0 0 0 0.5px var(--hairline),
      var(--shadow);
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .glabel {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink-3);
  }
  .tiny {
    font-size: 11px;
  }
  .group > .tiny {
    margin: 0;
  }
  .tiles {
    display: grid;
    gap: 6px;
  }
  .tiles.two {
    grid-template-columns: 1fr 1fr;
  }
  .tile {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    height: 38px;
    padding: 0 9px;
    border: 0;
    border-radius: 9px;
    background: var(--surface-2);
    box-shadow: inset 0 0 0 0.5px var(--hairline);
    color: var(--ink-2);
    font-size: 13px;
    font-weight: 500;
    text-align: left;
  }
  .tile > span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tile[aria-checked='true'] {
    background: var(--surface);
    color: var(--ink);
    box-shadow:
      inset 0 0 0 1.5px var(--accent),
      0 1px 2px rgba(0, 0, 0, 0.06);
  }
  .tile:disabled {
    opacity: 0.4;
  }
  .ratio {
    flex: none;
    height: 16px;
    max-width: 22px;
    border-radius: 2.5px;
    box-shadow: inset 0 0 0 1.5px currentColor;
    opacity: 0.75;
  }
  .caption {
    margin: -6px 0 0;
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
  }
  .theme {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: auto;
    padding: 0;
    border: 0;
    background: none;
    color: var(--ink-2);
  }
  .theme .tiny {
    max-width: 100%;
    line-height: 1.2;
    text-align: center;
  }
  .sample {
    position: relative;
    display: block;
    flex: none;
    width: 100%;
    aspect-ratio: 1;
    border-radius: 10px;
    box-shadow: inset 0 0 0 0.5px var(--hairline-strong);
    transition: box-shadow 160ms ease-out;
  }
  .sample .dot {
    position: absolute;
    right: 7px;
    bottom: 7px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  .theme[aria-checked='true'] {
    color: var(--ink);
  }
  .theme[aria-checked='true'] .sample {
    box-shadow:
      0 0 0 2px var(--surface),
      0 0 0 3.5px var(--accent);
  }
  .accents {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .accent {
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--c);
    box-shadow: inset 0 0 0 0.5px rgba(0, 0, 0, 0.18);
    transition:
      transform 160ms ease-out,
      box-shadow 160ms ease-out;
  }
  .accent.auto {
    background: conic-gradient(from 210deg, var(--c) 0 50%, var(--surface-2) 50% 100%);
  }
  .accent[aria-checked='true'] {
    box-shadow:
      0 0 0 2px var(--surface),
      0 0 0 3.5px var(--c);
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .info li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-height: 32px;
  }
  .actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .actions .btn {
    justify-content: center;
    height: 34px;
  }
  .actions .wide {
    grid-column: 1 / -1;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: -6px 0 0;
    color: var(--good-ink);
  }
  .state.bad {
    color: var(--bad-ink);
  }
  .state span {
    flex: 1;
    min-width: 0;
  }
  .open {
    height: 26px;
  }
  .spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-right-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .btn,
  .step,
  .tile,
  .theme {
    transition:
      transform 160ms ease-out,
      background var(--dur) var(--ease),
      box-shadow 160ms ease-out;
  }
  .btn:active:not(:disabled),
  .step:active:not(:disabled),
  .tile:active:not(:disabled),
  .theme:active,
  .accent:active,
  .thumb:active {
    transform: scale(0.97);
  }
  @media (hover: hover) and (pointer: fine) {
    .step:hover:not(:disabled) {
      background: var(--surface-press);
    }
    .tile:hover:not(:disabled):not([aria-checked='true']) {
      background: var(--surface-hover);
      color: var(--ink);
    }
    .thumb:hover {
      opacity: 0.9;
    }
  }
  @media (max-width: 920px) {
    .studio {
      grid-template-columns: 1fr;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .btn:active:not(:disabled),
    .step:active:not(:disabled),
    .tile:active:not(:disabled),
    .theme:active,
    .accent:active,
    .thumb:active {
      transform: none;
    }
  }
</style>
