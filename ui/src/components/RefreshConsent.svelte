<script lang="ts">
  import { t } from '../lib/i18n.svelte'
  import { answerConsent, refresh } from '../lib/refresh.svelte'

  let primary: HTMLButtonElement | undefined = $state()
  $effect(() => {
    if (refresh.asking) primary?.focus()
  })
  const keys = (e: KeyboardEvent) => e.key === 'Escape' && !refresh.busy && answerConsent(false)
</script>

<svelte:window onkeydown={(e) => refresh.asking && keys(e)} />

{#if refresh.asking}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={() => !refresh.busy && answerConsent(false)}></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="refresh-title" aria-describedby="refresh-text">
    <h2 id="refresh-title">{t('refresh.consent.title')}</h2>
    <p id="refresh-text">{t('refresh.consent.text')}</p>
    <div class="sample" aria-label={t('refresh.consent.sample')}>
      <span class="dim">⏺ Stop hook feedback</span>
      <span>⏺ {t('refresh.consent.reply')}</span>
    </div>
    <p class="note">{t('refresh.consent.note')}</p>
    {#if refresh.error}<p class="error" role="alert">{t('refresh.consent.failed', { why: refresh.error })}</p>{/if}
    <div class="actions">
      <button class="btn" disabled={refresh.busy} onclick={() => answerConsent(false)}>{t('common.cancel')}</button>
      <button class="btn primary" bind:this={primary} disabled={refresh.busy} onclick={() => answerConsent(true)}>{t('refresh.consent.ok')}</button>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: rgba(0, 0, 0, 0.32);
  }
  .dialog {
    position: fixed;
    z-index: 51;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(460px, calc(100vw - 32px));
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 22px 24px 20px;
    border-radius: 16px;
    background: var(--surface);
    border: 0.5px solid var(--hairline-strong);
    box-shadow: var(--shadow-pop);
  }
  h2 {
    font-size: 17px;
  }
  p {
    font-size: 13px;
    line-height: 1.55;
    color: var(--ink-2);
  }
  .sample {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border-radius: 9px;
    background: var(--surface-2);
    border: 0.5px solid var(--hairline);
    font-family: ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
    font-size: 12px;
  }
  .dim {
    color: var(--ink-3);
  }
  .note {
    font-size: 12px;
    color: var(--ink-3);
  }
  .error {
    color: var(--bad-ink);
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding-top: 2px;
  }
</style>
