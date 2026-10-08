<script lang="ts">
  import { api } from '../lib/api'
  import { refresh } from '../lib/store.svelte'
  import { t } from '../lib/i18n.svelte'

  let { kind, binary }: { kind: 'claude' | 'codex' | 'antigravity'; binary: string | null } = $props()

  const COMMANDS = { claude: ['claude', ''], codex: ['codex', ' login'], antigravity: ['agy', ''] } as const
  const windows = $derived(!!binary && /^[A-Za-z]:|\\/.test(binary))
  const command = $derived(COMMANDS[kind][0] + COMMANDS[kind][1])
  // the CLI a desktop app bundles is not on PATH
  const fullCommand = $derived(binary ? `${windows ? '& ' : ''}"${binary}"${COMMANDS[kind][1]}` : null)

  let busy = $state(false)
  let result = $state<{ ok: boolean; text: string } | null>(null)

  async function retry() {
    busy = true
    result = null
    try {
      await api.setCapture(kind, true)
      result = { ok: true, text: t('signin.ok') }
      await refresh()
    } catch {
      result = { ok: false, text: t('signin.stillOut') }
    } finally {
      busy = false
    }
  }
</script>

<div class="steps">
  <b class="small">{t('signin.title')}</b>
  <ol class="small">
    <li>{t('signin.open')}</li>
    <li>{t('signin.run')} <code>{command}</code></li>
    <li>{t(`signin.${kind}.account`)}</li>
    <li>{t('signin.done')}</li>
  </ol>
  {#if fullCommand}
    <p class="path subtle small">{t('signin.notFound')} <code>{fullCommand}</code></p>
  {/if}
  <div class="row">
    <button class="btn small" disabled={busy} onclick={retry}>{busy ? t('cap.working') : t('signin.retry')}</button>
    {#if result}<span class="small" class:bad={!result.ok} class:good={result.ok}>{result.text}</span>{/if}
  </div>
</div>

<style>
  .steps {
    margin-top: 10px;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--surface-2);
    border: 0.5px solid var(--hairline);
  }
  ol {
    margin: 6px 0 8px;
    padding-left: 20px;
    display: grid;
    gap: 4px;
  }
  code {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 11.5px;
    word-break: break-all;
  }
  .path {
    margin: 0 0 10px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .good {
    color: var(--good-ink);
  }
  .bad {
    color: var(--bad-ink);
  }
</style>
