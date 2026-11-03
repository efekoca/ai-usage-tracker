<script lang="ts">
  import { tick, untrack } from 'svelte'
  import { app, latest, saveSettings, setFilter } from '../lib/store.svelte'
  import { api, type ProjectRow, type SessionRow } from '../lib/api'
  import { NAV } from '../lib/nav'
  import { fmtDateTime, searchKey, t, toolLabel } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'

  let { open = $bindable(false) }: { open?: boolean } = $props()

  type Group = 'pages' | 'settings' | 'actions' | 'projects' | 'models' | 'sessions'
  type Item = { id: string; group: Group; label: string; detail?: string; icon: string; keys: string; run: () => void }

  const GROUPS: Group[] = ['pages', 'settings', 'actions', 'projects', 'models', 'sessions']
  // only these show before anything is typed; the rest would be a long list
  const BROWSE: Group[] = ['pages', 'actions']
  const PER_GROUP = 6

  let query = $state('')
  let active = $state(0)
  let input: HTMLInputElement | undefined = $state()
  let list: HTMLUListElement | undefined = $state()
  let sessions = $state<SessionRow[]>([])
  // every project and model, not only those in the current period and filter
  let projects = $state<ProjectRow[]>([])
  let models = $state<string[]>([])

  // "bütçe", "butce" and "BÜTÇE" all match
  const norm = (s: string) => searchKey(s).normalize('NFD').replace(/\p{M}/gu, '')

  function go(view: typeof app.view, section?: string) {
    app.view = view
    if (section) {
      // after the page has rendered and scrolled to its top
      tick().then(() => requestAnimationFrame(() => document.getElementById(`settings-${section}`)?.scrollIntoView({ block: 'start', behavior: 'smooth' })))
    }
  }

  const SETTINGS: [string, string][] = [
    ['settings.general', 'general'],
    ['settings.notify.title', 'notify'],
    ['settings.widget', 'widget'],
    ['settings.updates', 'updates'],
    ['settings.privacy', 'privacy'],
    ['settings.pricing', 'pricing'],
    ['settings.data', 'data'],
    ['settings.about', 'about'],
    ['settings.language', 'general'],
    ['settings.theme', 'general'],
    ['settings.accent', 'general'],
    ['settings.currency', 'general'],
    ['settings.budget', 'general'],
    ['limits.mode', 'general'],
    ['settings.primaryMetric', 'general'],
    ['settings.notify', 'notify'],
    ['settings.notify.test', 'notify'],
    ['settings.hotkey', 'widget'],
    ['settings.updates.auto', 'updates'],
    ['settings.hideProjects', 'privacy'],
  ]
  const sectionName: Record<string, string> = Object.fromEntries(SETTINGS.slice(0, 8).map(([k, s]) => [s, k]))

  const projectName = (s: SessionRow) => (s.hidden ? `${t('projects.hidden')}${s.project_id !== null ? ` #${s.project_id}` : ''}` : s.project || t('projects.noProject'))

  const items = $derived.by((): Item[] => {
    const out: Item[] = []
    for (const g of NAV) {
      for (const p of g.items) {
        const label = t(`nav.${p.id}`)
        out.push({ id: `page:${p.id}`, group: 'pages', label, icon: p.icon, keys: label, run: () => go(p.id) })
      }
    }
    const autostart = app.info && app.info.platform !== 'windows' ? 'settings.autostart.mac' : 'settings.autostart'
    // only rows the Settings page shows now
    const absent = new Set([
      ...(app.info?.accent_color ? [] : ['settings.accent']),
      ...(app.settings?.notify.enabled ? [] : ['settings.notify.test']),
      ...(app.update?.configured ? [] : ['settings.updates.auto']),
    ])
    const settingKeys: [string, string][] = [
      ...SETTINGS.filter(([key]) => !absent.has(key)),
      [autostart, 'general'],
      ...(app.info?.platform === 'macos' ? ([['settings.dock', 'general']] as [string, string][]) : []),
    ]
    for (const [key, section] of settingKeys) {
      const label = t(key)
      const isSection = sectionName[section] === key
      out.push({
        id: `setting:${key}`,
        group: 'settings',
        label,
        detail: isSection ? t('nav.settings') : `${t('nav.settings')} › ${t(sectionName[section])}`,
        icon: 'settings',
        keys: `${label} ${t(sectionName[section])}`,
        run: () => go('settings', section),
      })
    }
    const s = app.settings
    const action = (id: string, icon: string, run: () => void) => {
      const label = t(`palette.action.${id}`)
      out.push({ id: `action:${id}`, group: 'actions', label, icon, keys: label, run })
    }
    action('rescan', 'refresh', () => void api.rescan())
    if (s) {
      action('widget', 'widget', () => void api.setWidgetVisible(!s.widget.visible))
      for (const theme of ['light', 'dark', 'system'] as const) {
        if (s.theme !== theme) action(`theme.${theme}`, 'eye', () => void saveSettings({ theme }))
      }
    }
    action('updates', 'download', () => {
      go('settings', 'updates')
      api.checkUpdate().then((u) => (app.update = u)).catch(() => {})
    })
    const hideAll = !!s?.hide_project_names
    for (const p of projects) {
      const hidden = hideAll || p.hidden
      const label = hidden ? `${t('projects.hidden')} #${p.id}` : p.name
      out.push({
        id: `project:${p.id}`,
        group: 'projects',
        label,
        detail: t('palette.filterProject'),
        icon: 'projects',
        keys: hidden ? label : `${label} ${p.path}`,
        run: () => {
          setFilter({ ...app.filter, projects: [p.id] })
          go('overview')
        },
      })
    }
    for (const m of models) {
      out.push({
        id: `model:${m}`,
        group: 'models',
        label: m,
        detail: t('palette.filterModel'),
        icon: 'breakdown',
        keys: m,
        run: () => {
          setFilter({ ...app.filter, models: [m] })
          go('overview')
        },
      })
    }
    for (const r of sessions) {
      const name = projectName(r)
      out.push({
        id: `session:${r.tool}${r.session_id}`,
        group: 'sessions',
        label: `${name} · ${fmtDateTime(r.started_ms)}`,
        detail: [toolLabel(r.tool), ...r.models.slice(0, 2).map((m) => m.model)].join(' · '),
        icon: 'sessions',
        keys: [r.hidden ? name : r.project, r.session_id, ...r.models.map((m) => m.model)].join(' '),
        run: () => {
          app.openSession = r.tool + r.session_id
          go('sessions')
        },
      })
    }
    return out
  })

  const shown = $derived.by((): Item[] => {
    const q = norm(query.trim())
    const out: Item[] = []
    for (const g of GROUPS) {
      if (!q && !BROWSE.includes(g)) continue
      const scored = items
        .filter((i) => i.group === g)
        .map((i) => {
          const label = norm(i.label)
          const score = !q ? 0 : label.startsWith(q) ? 0 : label.split(/[\s·/›-]+/).some((w) => w.startsWith(q)) ? 1 : norm(i.keys).includes(q) ? 2 : -1
          return { i, score }
        })
        .filter((x) => x.score >= 0)
        .sort((a, b) => a.score - b.score)
      out.push(...scored.slice(0, q ? PER_GROUP : scored.length).map((x) => x.i))
    }
    return out
  })

  /** The shown items by group, each with its place in `shown`, which the arrow keys move through. */
  const groups = $derived.by(() => {
    const out: { group: Group; items: { item: Item; i: number }[] }[] = []
    shown.forEach((item, i) => {
      const last = out[out.length - 1]
      if (last?.group === item.group) last.items.push({ item, i })
      else out.push({ group: item.group, items: [{ item, i }] })
    })
    return out
  })

  $effect(() => {
    void query
    active = 0
  })

  $effect(() => {
    if (!open) return
    // however the palette closes, focus goes back to where it was
    const back = document.activeElement as HTMLElement | null
    query = ''
    active = 0
    sessions = []
    tick().then(() => input?.focus())
    const [period, filter] = untrack(() => [$state.snapshot(app.period), $state.snapshot(app.filter)])
    const stop = [
      latest(() => api.sessions(period, filter), (r) => (sessions = r.sessions.sort((a, b) => b.started_ms - a.started_ms).slice(0, 300))),
      latest(() => api.projects(), (p) => (projects = p)),
      latest(() => api.models(), (m) => (models = m)),
    ]
    return () => {
      stop.forEach((f) => f())
      back?.focus()
    }
  })

  function close() {
    open = false
  }

  function choose(i: Item | undefined) {
    if (!i) return
    open = false
    i.run()
  }

  function keydown(e: KeyboardEvent) {
    // keys that finish an IME composition belong to the input method
    if (e.isComposing) return
    const n = shown.length
    if (e.key === 'ArrowDown' && n) {
      e.preventDefault()
      active = (active + 1) % n
    } else if (e.key === 'ArrowUp' && n) {
      e.preventDefault()
      active = (active - 1 + n) % n
    } else if (e.key === 'Home' && n) {
      e.preventDefault()
      active = 0
    } else if (e.key === 'End' && n) {
      e.preventDefault()
      active = n - 1
    } else if (e.key === 'Enter') {
      e.preventDefault()
      choose(shown[active])
    } else if (e.key === 'Escape') {
      e.preventDefault()
      close()
    } else if (e.key === 'Tab') {
      // focus stays in the search field; the list is reached with the arrow keys
      e.preventDefault()
    }
  }

  $effect(() => {
    void active
    list?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' })
  })
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={close}></div>
  <!-- a click inside keeps the focus in the search field, so the keys keep working -->
  <div class="palette" role="dialog" tabindex="-1" aria-modal="true" aria-label={t('palette.label')} onmousedown={(e) => e.target !== input && e.preventDefault()}>
    <div class="search">
      <Icon name="search" size={16} />
      <input
        bind:this={input}
        bind:value={query}
        onkeydown={keydown}
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-list"
        aria-autocomplete="list"
        aria-activedescendant={shown.length ? `palette-${active}` : undefined}
        aria-label={t('palette.label')}
        placeholder={t('palette.placeholder')}
        spellcheck="false"
        autocomplete="off"
      />
    </div>
    <ul id="palette-list" role="listbox" aria-label={t('palette.label')} bind:this={list}>
      {#each groups as g (g.group)}
        <li role="group" aria-labelledby="palette-group-{g.group}">
          <div class="group" id="palette-group-{g.group}">{t(`palette.group.${g.group}`)}</div>
          <ul role="presentation">
            {#each g.items as { item, i } (item.id)}
              <!-- the search field moves through the options (aria-activedescendant), so they take no focus -->
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_interactive_supports_focus -->
              <li
                id="palette-{i}"
                data-index={i}
                role="option"
                aria-selected={i === active}
                class:active={i === active}
                onmousemove={() => (active = i)}
                onclick={() => choose(item)}
              >
                <Icon name={item.icon} size={15} />
                <span class="label">{item.label}</span>
                {#if item.detail}<span class="detail">{item.detail}</span>{/if}
              </li>
            {/each}
          </ul>
        </li>
      {/each}
    </ul>
    {#if !shown.length}<p class="empty" role="status">{t('palette.empty')}</p>{/if}
    <div class="hint small subtle">{t('palette.hint')}</div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: rgba(0, 0, 0, 0.32);
  }
  .palette {
    position: fixed;
    z-index: 51;
    top: 12vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(640px, calc(100vw - 32px));
    max-height: 64vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--hairline);
    color: var(--ink-2);
  }
  input {
    flex: 1;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--ink);
    font: inherit;
    font-size: 15px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 6px;
    overflow-y: auto;
  }
  li[role='group'] > ul {
    padding: 0;
    overflow: visible;
  }
  li[role='option'] {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    color: var(--ink);
    cursor: default;
  }
  li.active {
    background: var(--surface-hover);
  }
  .label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .detail {
    margin-left: auto;
    padding-left: 12px;
    color: var(--ink-3);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 45%;
  }
  .group {
    padding: 10px 10px 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--ink-3);
  }
  .empty {
    margin: 0;
    padding: 18px 10px;
    color: var(--ink-3);
    text-align: center;
  }
  .hint {
    padding: 8px 14px;
    border-top: 1px solid var(--hairline);
  }
</style>
