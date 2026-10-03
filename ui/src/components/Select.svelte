<script lang="ts" module>
  export type SelectOption = { value: string; label: string; sub?: string; disabled?: boolean }
</script>

<script lang="ts">
  // Drop-down picker that matches the app (the native one does not): a trigger button and a
  // floating list with an optional search field, full keyboard support (arrows, Home/End,
  // Enter, Escape, type-to-find) and the listbox pattern for screen readers. The list floats
  // above everything, so a scrolling card never clips it.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { i18n, t } from '../lib/i18n.svelte'

  let {
    value = $bindable(''),
    options,
    label,
    onchange,
    searchable = undefined,
    disabled = false,
    placeholder = '',
    minWidth = 0,
    id = undefined,
  }: {
    value?: string
    options: SelectOption[]
    /** Accessible name (shown to screen readers; visible labels stay outside). */
    label: string
    onchange?: (value: string) => void
    /** Show a search field; by default when there are more than eight options. */
    searchable?: boolean
    disabled?: boolean
    placeholder?: string
    minWidth?: number
    id?: string
  } = $props()

  const uid = `sel-${Math.random().toString(36).slice(2, 9)}`
  let open = $state(false)
  let query = $state('')
  let active = $state(0)
  let trigger: HTMLButtonElement | undefined = $state()
  let list: HTMLUListElement | undefined = $state()
  let search: HTMLInputElement | undefined = $state()
  let pos = $state({ left: 0, top: 0, width: 0, maxHeight: 300, up: false })

  const withSearch = $derived(searchable ?? options.length > 8)
  const current = $derived(options.find((o) => o.value === value))
  const fold = (s: string) => s.toLocaleLowerCase(i18n.lang === 'tr' ? 'tr-TR' : 'en-US')
  const shown = $derived(query.trim() ? options.filter((o) => fold(`${o.label} ${o.sub ?? ''}`).includes(fold(query.trim()))) : options)

  function place() {
    if (!trigger) return
    const r = trigger.getBoundingClientRect()
    const below = window.innerHeight - r.bottom - 8
    const above = r.top - 8
    const up = below < 220 && above > below
    const width = Math.max(r.width, minWidth, 180)
    pos = {
      left: Math.min(r.left, window.innerWidth - width - 8),
      top: up ? r.top - 4 : r.bottom + 4,
      width,
      maxHeight: Math.max(140, Math.min(320, up ? above : below)),
      up,
    }
  }
  async function show() {
    if (disabled || open) return
    query = ''
    place()
    open = true
    active = Math.max(0, shown.findIndex((o) => o.value === value))
    await tick()
    // the list is as wide as its longest option: keep it inside the window
    const pop = document.getElementById(`${uid}-pop`)
    if (pop) {
      const w = pop.getBoundingClientRect().width
      if (pos.left + w > window.innerWidth - 8) pos = { ...pos, left: Math.max(8, window.innerWidth - 8 - w) }
    }
    if (withSearch) search?.focus()
    else list?.focus()
    scrollActive()
  }
  function hide(refocus = true) {
    if (!open) return
    open = false
    if (refocus) trigger?.focus()
  }
  function choose(o: SelectOption | undefined) {
    if (!o || o.disabled) return
    if (o.value !== value) {
      value = o.value
      onchange?.(o.value)
    }
    hide()
  }
  async function scrollActive() {
    await tick()
    list?.querySelector<HTMLElement>(`#${uid}-o${active}`)?.scrollIntoView({ block: 'nearest' })
  }
  function move(step: number) {
    if (!shown.length) return
    let i = active
    for (let n = 0; n < shown.length; n++) {
      i = (i + step + shown.length) % shown.length
      if (!shown[i].disabled) break
    }
    active = i
    scrollActive()
  }

  let typed = ''
  let typedAt = 0
  function onListKey(e: KeyboardEvent) {
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        move(1)
        break
      case 'ArrowUp':
        e.preventDefault()
        move(-1)
        break
      case 'Home':
        if (withSearch && e.target === search) return
        e.preventDefault()
        active = 0
        scrollActive()
        break
      case 'End':
        if (withSearch && e.target === search) return
        e.preventDefault()
        active = shown.length - 1
        scrollActive()
        break
      case 'Enter':
        e.preventDefault()
        choose(shown[active])
        break
      case 'Escape':
        e.preventDefault()
        hide()
        break
      case 'Tab':
        hide(false)
        break
      default:
        // type to find, when there is no search field
        if (!withSearch && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          const now = Date.now()
          typed = now - typedAt > 700 ? e.key : typed + e.key
          typedAt = now
          const i = shown.findIndex((o) => fold(o.label).startsWith(fold(typed)))
          if (i >= 0) {
            active = i
            scrollActive()
          }
        }
    }
  }
  function onTriggerKey(e: KeyboardEvent) {
    if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) {
      e.preventDefault()
      show()
    }
  }
  $effect(() => {
    if (!open) return
    // a click anywhere else, a resize or a scroll outside the list closes it
    const outside = (e: PointerEvent) => {
      const el = e.target as Node
      if (!trigger?.contains(el) && !document.getElementById(`${uid}-pop`)?.contains(el)) hide(false)
    }
    const away = (e: Event) => {
      if (!document.getElementById(`${uid}-pop`)?.contains(e.target as Node)) hide(false)
    }
    const resize = () => hide(false)
    document.addEventListener('pointerdown', outside, true)
    document.addEventListener('scroll', away, true)
    window.addEventListener('resize', resize)
    return () => {
      document.removeEventListener('pointerdown', outside, true)
      document.removeEventListener('scroll', away, true)
      window.removeEventListener('resize', resize)
    }
  })
  $effect(() => {
    // keep the highlighted row inside the filtered list
    void query
    if (active >= shown.length) active = Math.max(0, shown.length - 1)
  })
</script>

<button
  bind:this={trigger}
  {id}
  type="button"
  class="trigger"
  class:open
  role="combobox"
  aria-label={label}
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-controls="{uid}-list"
  {disabled}
  style:min-width={minWidth ? `${minWidth}px` : undefined}
  onclick={() => (open ? hide() : show())}
  onkeydown={onTriggerKey}
>
  <span class="value" class:placeholder={!current}>{current ? current.label : placeholder || '—'}</span>
  <span class="chev" aria-hidden="true"><Icon name="chevron" size={14} /></span>
</button>

{#if open}
  <div
    id="{uid}-pop"
    class="pop"
    class:up={pos.up}
    style="left:{pos.left}px; top:{pos.top}px; min-width:{pos.width}px; max-height:{pos.maxHeight}px"
  >
    {#if withSearch}
      <div class="search">
        <Icon name="search" size={14} />
        <input
          bind:this={search}
          bind:value={query}
          type="text"
          placeholder={t('common.search')}
          aria-label={t('common.search')}
          aria-controls="{uid}-list"
          aria-activedescendant={shown.length ? `${uid}-o${active}` : undefined}
          onkeydown={onListKey}
          autocomplete="off"
          spellcheck="false"
        />
      </div>
    {/if}
    <ul
      bind:this={list}
      id="{uid}-list"
      role="listbox"
      aria-label={label}
      tabindex="-1"
      aria-activedescendant={shown.length ? `${uid}-o${active}` : undefined}
      onkeydown={onListKey}
    >
      {#each shown as o, i (o.value)}
        <!-- keys are handled by the listbox (aria-activedescendant); a click picks the option -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id="{uid}-o{i}"
          role="option"
          aria-selected={o.value === value}
          aria-disabled={o.disabled || undefined}
          class:active={i === active}
          class:selected={o.value === value}
          class:disabled={o.disabled}
          onpointerenter={() => (active = i)}
          onpointerdown={(e) => e.preventDefault()}
          onclick={() => choose(o)}
        >
          <span class="check" aria-hidden="true">{#if o.value === value}<Icon name="check" size={14} />{/if}</span>
          <span class="txt">
            <span class="lbl">{o.label}</span>
            {#if o.sub}<span class="sub">{o.sub}</span>{/if}
          </span>
        </li>
      {:else}
        <li class="none" role="presentation">{t('common.noResults')}</li>
      {/each}
    </ul>
  </div>
{/if}

<style>
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    max-width: 100%;
    padding: 0 8px 0 11px;
    border-radius: 8px;
    border: 0.5px solid var(--hairline-strong);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 13px;
    text-align: left;
    box-shadow: 0 0.5px 1px rgba(0, 0, 0, 0.04);
    transition:
      border-color var(--dur) var(--ease),
      background var(--dur) var(--ease);
  }
  .trigger:hover:not(:disabled) {
    background: var(--surface-2);
    border-color: var(--axis);
  }
  .trigger.open {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .trigger:disabled {
    opacity: 0.5;
  }
  .value {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .placeholder {
    color: var(--ink-3);
  }
  .chev {
    display: grid;
    place-items: center;
    color: var(--ink-3);
    transform: rotate(90deg);
    transition: transform var(--dur) var(--ease);
  }
  .open .chev {
    transform: rotate(-90deg);
  }
  .pop {
    position: fixed;
    width: max-content;
    max-width: min(420px, calc(100vw - 16px));
    z-index: 100;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 0.5px solid var(--hairline-strong);
    border-radius: 11px;
    box-shadow: var(--shadow-pop);
    overflow: hidden;
    animation: drop 120ms var(--ease);
  }
  .pop.up {
    transform: translateY(-100%);
    animation-name: lift;
  }
  @keyframes drop {
    from {
      opacity: 0;
      margin-top: -4px;
    }
  }
  @keyframes lift {
    from {
      opacity: 0;
      margin-top: 4px;
    }
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border-bottom: 0.5px solid var(--hairline);
    color: var(--ink-3);
    flex: none;
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 36px;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--ink);
    font-size: 13px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 4px;
    overflow-y: auto;
    outline: none;
    flex: 1;
    min-height: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 30px;
    padding: 5px 8px 5px 4px;
    border-radius: 7px;
    font-size: 13px;
    color: var(--ink);
  }
  li.active {
    background: var(--surface-hover);
  }
  li.selected .lbl {
    font-weight: 600;
  }
  li.disabled {
    opacity: 0.45;
  }
  .check {
    width: 16px;
    flex: none;
    display: grid;
    place-items: center;
    color: var(--accent);
  }
  .txt {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .lbl,
  .sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: 11.5px;
    color: var(--ink-3);
  }
  li.none {
    color: var(--ink-3);
    padding: 8px 10px;
  }
  @media (prefers-reduced-motion: reduce) {
    .pop {
      animation: none;
    }
  }
</style>
