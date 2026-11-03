<script lang="ts">
  import { app, latest } from '../lib/store.svelte'
  import { api, providersOf, type Period, type PlansFile } from '../lib/api'
  import { saveAs } from '../lib/dialogs'
  import { fmtDate, i18n, localDate, t } from '../lib/i18n.svelte'
  import { planMultiple } from '../lib/plans'
  import { buildDay, buildMonth, buildYear, type Card, type CardKind } from '../lib/cards'
  import { drawCard, SIZE, type Shape, type Theme } from '../lib/cardCanvas'
  import Segmented from '../components/Segmented.svelte'
  import Icon from '../components/Icon.svelte'

  const SHAPES: Shape[] = ['story', 'square']
  let kind = $state<CardKind>('month')
  let theme = $state<Theme>('dark')
  /** Any day inside the chosen day, month or year. */
  let at = $state(new Date())
  /** `undefined` while loading, `null` for a period without use. */
  let card = $state.raw<Card | null | undefined>(undefined)
  let failed = $state('')
  let busy = $state<Shape | null>(null)
  let status = $state<{ shape: Shape; ok: boolean; text: string } | null>(null)
  const canvases: Partial<Record<Shape, HTMLCanvasElement>> = $state({})
  let plans: PlansFile | null = null
  /** The period on screen, so a data refresh keeps the cards and the saved note in place. */
  let shown = ''

  const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n)
  function bounds(k: CardKind, d: Date) {
    if (k === 'day') return { from: new Date(d.getFullYear(), d.getMonth(), d.getDate()), to: new Date(d.getFullYear(), d.getMonth(), d.getDate()) }
    if (k === 'month') return { from: new Date(d.getFullYear(), d.getMonth(), 1), to: new Date(d.getFullYear(), d.getMonth() + 1, 0) }
    return { from: new Date(d.getFullYear(), 0, 1), to: new Date(d.getFullYear(), 11, 31) }
  }
  const moved = (k: CardKind, d: Date, n: number) =>
    k === 'day' ? addDays(d, n) : k === 'month' ? new Date(d.getFullYear(), d.getMonth() + n, 1) : new Date(d.getFullYear() + n, 0, 1)
  const range = $derived(bounds(kind, at))
  const canNext = $derived(bounds(kind, moved(kind, at, 1)).from <= new Date())
  const label = $derived(kind === 'day' ? fmtDate(at, 'long') : kind === 'month' ? fmtDate(at, 'monthLong') : String(at.getFullYear()))
  const stem = $derived(kind === 'day' ? localDate(range.from) : kind === 'month' ? localDate(range.from).slice(0, 7) : String(range.from.getFullYear()))

  $effect(() => {
    void app.tick
    void i18n.lang
    void i18n.currency
    const k = kind
    const { from, to } = range
    const period: Period = { kind: 'custom', from: localDate(from), to: localDate(to) }
    if (shown !== `${k}:${period.from}`) {
      shown = `${k}:${period.from}`
      card = undefined
      status = null
    }
    return latest(
      async () => {
        if (k === 'day') {
          const [detail, prior] = await Promise.all([
            api.dayDetail(localDate(from)),
            api.report({ kind: 'custom', from: localDate(addDays(from, -30)), to: localDate(addDays(from, -1)) }),
          ])
          return buildDay(detail, prior, from)
        }
        const [report, value, p, sessions] = await Promise.all([
          api.report(period),
          api.planValue(period),
          plans ?? api.plans(),
          k === 'month' ? api.sessions(period) : null,
        ])
        plans = p
        const multiple = planMultiple({ plans: p, settings: app.settings, accounts: app.accounts, value, fromFirstSeen: true }, providersOf(app.settings?.enabled_sources ?? []))
        return sessions ? buildMonth(report, sessions, from, value.dates.length, multiple) : buildYear(report, from.getFullYear(), value.dates.length, multiple)
      },
      (c) => {
        card = c
        failed = ''
      },
      (e) => {
        card = null
        failed = String(e)
      },
    )
  })

  $effect(() => {
    if (!card) return
    const family = getComputedStyle(document.body).fontFamily
    for (const s of SHAPES) {
      const c = canvases[s]
      if (c) drawCard(c, card, s, theme, family)
    }
  })

  function pick(k: CardKind) {
    kind = k
    at = new Date()
  }

  async function download(shape: Shape) {
    const canvas = canvases[shape]
    if (!canvas || !card) return
    status = null
    const chosen = await saveAs(`AI-Usage_${stem}${shape === 'square' ? '_square' : ''}.png`, 'PNG', 'png')
    if (!chosen) return
    busy = shape
    try {
      const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, 'image/png'))
      if (!blob) throw new Error('png')
      await api.saveImage(chosen, new Uint8Array(await blob.arrayBuffer()))
      status = { shape, ok: true, text: t('cards.saved') }
    } catch (e) {
      status = { shape, ok: false, text: t('cards.failed', { e: String(e) }) }
    } finally {
      busy = null
    }
  }
</script>

<header class="bar">
  <h1>{t('nav.cards')}</h1>
</header>
<p class="subtle small help">{t('cards.help')}</p>

<div class="controls">
  <div class="period">
    <Segmented label={t('cards.period')} value={kind} options={(['day', 'month', 'year'] as const).map((k) => ({ value: k, label: t(`cards.kind.${k}`) }))} onchange={pick} />
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
  <Segmented
    label={t('cards.theme')}
    bind:value={theme}
    options={[
      { value: 'dark', label: t('cards.theme.dark'), swatch: '#0b1020' },
      { value: 'light', label: t('cards.theme.light'), swatch: '#f4f5f8' },
    ]}
  />
</div>

<div class="stage">
  {#if failed}
    <p class="note muted small"><Icon name="warning" size={15} />{t('common.error', { e: failed })}</p>
  {:else if card === null}
    <p class="note muted small"><Icon name="info" size={15} />{t('cards.empty')}</p>
  {:else}
    {#each SHAPES as s (s)}
      <figure class={s}>
        <div class="frame" class:loading={!card}>
          <canvas bind:this={canvases[s]} class:hidden={!card}>{t('cards.aria', { title: label })}</canvas>
        </div>
        <figcaption>
          <span class="meta">
            <span class="name">{t(`cards.shape.${s}`)}</span>
            {#if status?.shape === s}
              <span class="small state" class:bad={!status.ok} role={status.ok ? 'status' : 'alert'}>
                <Icon name={status.ok ? 'check' : 'warning'} size={13} />{status.text}
              </span>
            {:else}
              <span class="small subtle num">{SIZE[s].w} × {SIZE[s].h}</span>
            {/if}
          </span>
          {#if status?.shape === s && status.ok}
            <button class="btn ghost" onclick={() => api.openLastImage()}>{t('cards.open')}</button>
          {/if}
          <button class="btn" class:primary={s === 'story'} onclick={() => download(s)} disabled={!card || busy !== null}>
            {#if busy === s}<span class="spin" aria-hidden="true"></span>{:else}<Icon name="download" size={15} />{/if}{t('cards.save')}
          </button>
        </figcaption>
      </figure>
    {/each}
  {/if}
</div>

<style>
  .help {
    margin: 4px 0 18px;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 16px;
  }
  .period {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
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
    min-width: 112px;
    padding: 0 6px;
    text-align: center;
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .stage {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: center;
    gap: 32px 40px;
    min-height: 420px;
    padding: 32px 24px 28px;
    border-radius: 16px;
    background: var(--stage);
    box-shadow: inset 0 0 0 0.5px var(--hairline);
  }
  .note {
    display: flex;
    align-items: center;
    gap: 8px;
    align-self: center;
  }
  figure {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin: 0;
    min-width: 0;
  }
  figure.story {
    width: min(306px, 100%);
  }
  figure.square {
    width: min(408px, 100%);
  }
  .frame {
    overflow: hidden;
    border-radius: 16px;
    box-shadow:
      0 0 0 0.5px var(--hairline-strong),
      var(--shadow-pop);
  }
  .story .frame {
    aspect-ratio: 9 / 16;
  }
  .square .frame {
    aspect-ratio: 1;
  }
  .frame.loading {
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
  figcaption {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .meta {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-size: 13px;
    font-weight: 600;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--good-ink);
  }
  .state.bad {
    color: var(--bad-ink);
  }
  .btn,
  .step {
    transition:
      transform 160ms ease-out,
      background var(--dur) var(--ease);
  }
  .btn:active:not(:disabled),
  .step:active:not(:disabled) {
    transform: scale(0.97);
  }
  @media (hover: hover) and (pointer: fine) {
    .step:hover:not(:disabled) {
      background: var(--surface-press);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .btn:active:not(:disabled),
    .step:active:not(:disabled) {
      transform: none;
    }
  }
</style>
