<script lang="ts">
  import { flip } from 'svelte/animate'
  import type { PanelData, PanelModule as Module, Settings } from '../../lib/api'
  import { t, type LimitMode } from '../../lib/i18n.svelte'
  import Icon from '../Icon.svelte'
  import PanelModule, { moduleKey } from './PanelModule.svelte'

  type Edit = {
    selected: string | null
    /** The dragged module's place while it is over the panel. */
    placeholder: string | null
    /** The dragged module's old place while it is outside the panel. */
    lifted: string | null
    name: (m: Module) => string
    onselect: (i: number) => void
    onremove: (i: number) => void
    ongrab: (e: PointerEvent, i: number) => void
    onkey: (e: KeyboardEvent, i: number) => void
  }
  let {
    modules,
    data,
    settings,
    now,
    edit = null,
    scanned = false,
    onaction,
  }: { modules: Module[]; data: PanelData | null; settings: Settings; now: number; edit?: Edit | null; scanned?: boolean; onaction?: (a: string) => void } = $props()

  // the panel's own used/remaining choice, or the app's
  const mode = $derived<LimitMode>(settings.panel.value === 'used' || settings.panel.value === 'remaining' ? settings.panel.value : settings.limit_display === 'remaining' ? 'remaining' : 'used')

  const reduce = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches
</script>

<div class="cc-grid" data-zone="panel" role={edit ? 'list' : undefined} aria-label={edit ? t('menubar.panel') : undefined}>
  {#each modules as m, i (moduleKey(m))}
    {@const k = moduleKey(m)}
    <div
      class="cell {m.size}"
      class:editing={!!edit}
      class:selected={edit?.selected === k}
      class:placeholder={edit?.placeholder === k}
      class:lifted={edit?.lifted === k}
      data-tile
      data-key={k}
      data-index={i}
      role={edit ? 'listitem' : undefined}
      animate:flip={{ duration: reduce ? 0 : 200 }}
    >
      <div class="body" inert={!!edit}>
        <PanelModule {m} {data} {settings} {now} {mode} {scanned} edit={!!edit} {onaction} />
      </div>
      {#if edit}
        <button class="hit" aria-label={edit.name(m)} aria-pressed={edit.selected === k} onpointerdown={(e) => edit.ongrab(e, i)} onclick={() => edit.onselect(i)} onkeydown={(e) => edit.onkey(e, i)}></button>
        <button class="minus" aria-label={t('menubar.panel.remove', { name: edit.name(m) })} title={t('menubar.panel.remove', { name: edit.name(m) })} onclick={() => edit.onremove(i)}>
          <Icon name="close" size={8} />
        </button>
      {/if}
    </div>
  {/each}
</div>

<style>
  .cc-grid {
    --cell: 64px;
    display: grid;
    grid-template-columns: repeat(4, var(--cell));
    grid-auto-rows: var(--cell);
    grid-auto-flow: row dense;
    gap: 8px;
    -webkit-user-select: none;
    user-select: none;
  }
  .cell {
    position: relative;
    min-width: 0;
  }
  .cell.w {
    grid-column: span 2;
  }
  .cell.m {
    grid-column: span 2;
    grid-row: span 2;
  }
  .cell.l {
    grid-column: span 4;
  }
  .body {
    width: 100%;
    height: 100%;
    transition: transform 160ms var(--ease), opacity 160ms var(--ease);
  }
  .cell.placeholder::before {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: 16px;
    border: 1.5px dashed color-mix(in srgb, var(--accent) 70%, transparent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .cell.s.placeholder::before {
    inset: 6px;
    border-radius: 50%;
  }
  .cell.placeholder .body,
  .cell.placeholder .minus {
    visibility: hidden;
  }
  .cell.lifted .body {
    opacity: 0.25;
    transform: scale(0.94);
  }
  .hit {
    position: absolute;
    inset: 0;
    padding: 0;
    border: 0;
    border-radius: 16px;
    background: transparent;
    cursor: grab;
    touch-action: none;
  }
  .cell.s .hit {
    inset: 6px;
    border-radius: 50%;
  }
  .hit:active {
    cursor: grabbing;
  }
  .hit:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .cell.editing:hover .body {
    transform: scale(1.025);
  }
  .cell.selected .hit {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .minus {
    position: absolute;
    top: -5px;
    left: -5px;
    z-index: 1;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0.5px solid var(--hairline-strong);
    border-radius: 50%;
    background: var(--surface);
    color: var(--ink-2);
    box-shadow: var(--shadow);
    opacity: 0;
    transform: scale(0.8);
    transition: opacity 140ms var(--ease), transform 140ms var(--ease), background 140ms var(--ease);
  }
  .cell.s .minus {
    top: 1px;
    left: 1px;
  }
  .cell:hover .minus,
  .cell:focus-within .minus,
  .cell.selected .minus {
    opacity: 1;
    transform: none;
  }
  .minus:hover,
  .minus:focus-visible {
    background: var(--critical);
    color: #fff;
  }
  @media (prefers-reduced-motion: reduce) {
    .body,
    .minus {
      transition: none;
    }
  }
</style>
