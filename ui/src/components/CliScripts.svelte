<script lang="ts">
  import { t } from '../lib/i18n.svelte'
  import { cliScripts, type ScriptKind } from '../lib/cliScripts'
  import Icon from './Icon.svelte'

  let { command, platform }: { command: string; platform: string } = $props()

  const tabs = $derived(cliScripts(command, platform))
  let picked: ScriptKind = $state('command')
  const tab = $derived(tabs.find((x) => x.kind === picked) ?? tabs[0])
  let buttons: HTMLButtonElement[] = $state([])

  function select(i: number) {
    const next = tabs[(i + tabs.length) % tabs.length]
    picked = next.kind
    copied = -1
    buttons[tabs.indexOf(next)]?.focus()
  }
  function key(e: KeyboardEvent, i: number) {
    const to = e.key === 'ArrowRight' ? i + 1 : e.key === 'ArrowLeft' ? i - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? tabs.length - 1 : null
    if (to === null) return
    e.preventDefault()
    select(to)
  }

  let copied = $state(-1)
  let timer: ReturnType<typeof setTimeout> | undefined
  async function copy(i: number, code: string) {
    try {
      await navigator.clipboard.writeText(code)
      copied = i
      clearTimeout(timer)
      timer = setTimeout(() => (copied = -1), 1500)
    } catch {
      /* clipboard unavailable */
    }
  }

  const lines = (code: string) => code.split('\n').map((text) => ({ text, comment: text.trimStart().startsWith('#') }))
  /** `text` in a hint is set as code. */
  const marked = (s: string) => s.split('`').map((text, i) => ({ text, code: i % 2 === 1 }))
  const hint = $derived(
    (['tmux', 'sketchybar', 'raycast'].includes(tab.kind)
      ? [t(`settings.cli.hint.${tab.kind}`), t('settings.cli.behind'), t(platform === 'macos' ? 'settings.cli.jq.macos' : 'settings.cli.jq.linux')]
      : [t(`settings.cli.hint.${tab.kind}`)]
    ).join(' '),
  )
</script>

<div class="term">
  <div class="tabs" role="tablist" aria-label={t('settings.cli')}>
    {#each tabs as x, i (x.kind)}
      <button
        bind:this={buttons[i]}
        role="tab"
        id="cli-tab-{x.kind}"
        aria-selected={x.kind === tab.kind}
        aria-controls="cli-panel"
        tabindex={x.kind === tab.kind ? 0 : -1}
        onclick={() => select(i)}
        onkeydown={(e) => key(e, i)}>{x.kind === 'command' ? t('settings.cli.tab.command') : x.label}</button
      >
    {/each}
  </div>
  <div class="panel" role="tabpanel" id="cli-panel" aria-labelledby="cli-tab-{tab.kind}">
    {#each tab.files as f, i (tab.kind + f.name)}
      <div class="file">
        <div class="bar">
          <span class="name">{f.name}</span>
          <button class="copy" class:done={copied === i} onclick={() => copy(i, f.code)} aria-label="{t('settings.cli.copy')}: {f.name}">
            <Icon name={copied === i ? 'check' : 'copy'} size={13} />{copied === i ? t('settings.cli.copied') : t('settings.cli.copy')}
          </button>
        </div>
        <pre><code>{#each lines(f.code) as l, n}{#if n}{'\n'}{/if}<span class:comment={l.comment}>{l.text}</span>{/each}</code></pre>
      </div>
    {/each}
  </div>
</div>
<p class="hint subtle small">{#each marked(hint) as p}{#if p.code}<code>{p.text}</code>{:else}{p.text}{/if}{/each}</p>

<style>
  .term {
    margin-top: 10px;
    border: 0.5px solid var(--hairline-strong);
    border-radius: 10px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    padding: 0 8px;
    border-bottom: 0.5px solid var(--hairline);
  }
  .tabs button {
    height: 34px;
    padding: 0 9px;
    margin-bottom: -0.5px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--ink-3);
    font-size: 12.5px;
    font-weight: 500;
    transition: color var(--dur) var(--ease);
  }
  .tabs button:hover {
    color: var(--ink-2);
  }
  .tabs button[aria-selected='true'] {
    color: var(--ink);
    border-bottom-color: var(--accent);
  }
  .file + .file {
    border-top: 0.5px solid var(--hairline);
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 6px 0 14px;
  }
  .name {
    font: 11.5px ui-monospace, 'Cascadia Mono', Consolas, monospace;
    color: var(--ink-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .copy {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex: none;
    height: 24px;
    padding: 0 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-2);
    font-size: 12px;
    font-weight: 500;
    transition: background var(--dur) var(--ease);
  }
  .copy:hover {
    background: var(--surface-hover);
    color: var(--ink);
  }
  .copy.done {
    color: var(--good-ink);
  }
  pre {
    margin: 0;
    padding: 4px 14px 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: 12px/1.6 ui-monospace, 'Cascadia Mono', Consolas, monospace;
    color: var(--ink);
    tab-size: 2;
  }
  .comment {
    color: var(--ink-3);
  }
  .hint {
    margin: 8px 0 0;
  }
  .hint code {
    font: 11.5px ui-monospace, 'Cascadia Mono', Consolas, monospace;
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--surface-2);
    white-space: nowrap;
  }
</style>
