<script lang="ts">
  import { onMount } from 'svelte'
  import { app, saveSettings } from '../lib/store.svelte'
  import { api, type PlansFile, type Provider, type SourceId, type SourceInfo } from '../lib/api'
  import { i18n, t } from '../lib/i18n.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Select from '../components/Select.svelte'
  import Icon from '../components/Icon.svelte'

  let sources: SourceInfo[] = $state([])
  let enabled: Record<string, boolean> = $state({})
  let plans: PlansFile | null = $state(null)
  let chosen: Record<string, string> = $state({ ...(app.settings?.plans ?? {}) })
  let showPrivacy = $state(false)
  let busy = $state(false)

  onMount(async () => {
    const [found, p] = await Promise.all([api.detectSources(), api.plans()])
    // fill toggles before the list renders so every bind:checked starts defined
    enabled = Object.fromEntries(found.map((s) => [s.id, s.supported && s.found]))
    plans = p
    sources = found
  })

  const anyFound = $derived(sources.some((s) => s.found && s.supported))
  const providers = $derived.by(() => {
    const p = new Set<Provider>()
    for (const s of sources) {
      if (!enabled[s.id]) continue
      if (s.id === 'codex') p.add('openai')
      else if (s.id !== 'chatgpt_desktop') p.add('anthropic')
    }
    return [...p]
  })

  async function start() {
    busy = true
    const list = sources.filter((s) => s.supported && enabled[s.id]).map((s) => s.id) as SourceId[]
    const planMap: Record<string, string> = {}
    for (const [k, v] of Object.entries(chosen)) if (v) planMap[k] = v
    await saveSettings({ enabled_sources: list, plans: planMap, onboarded: true })
    busy = false
  }
</script>

<main class="onb">
  <div class="panel">
    <div class="hero">
      <img src="/app-icon.png" alt="" width="56" height="56" />
      <h1>{t('onb.welcome')}</h1>
      <p class="muted lead">{t('onb.lead')}</p>
      <div class="lang"><Select label={t('settings.language')} value={app.settings?.language ?? 'system'} minWidth={170} options={[{ value: 'system', label: t('settings.language.system') }, { value: 'tr', label: 'Türkçe' }, { value: 'en', label: 'English' }]} onchange={(v) => saveSettings({ language: v as 'system' | 'tr' | 'en' })} /></div>
    </div>

    <section class="card">
      <h2>{t('onb.detected')}</h2>
      {#if sources.length === 0}
        <p class="muted">{t('common.loading')}</p>
      {/if}
      {#each sources as s (s.id)}
        <div class="list-row">
          <div class="state" class:found={s.found} aria-hidden="true"><Icon name={s.found ? 'check' : 'close'} size={14} /></div>
          <div class="body">
            <div><b>{t(`source.${s.id}`)}</b> <span class="subtle small">· {s.found ? (s.supported ? t('source.found') : t('source.unsupported')) : t('source.notFound')}</span></div>
            <div class="subtle small">{t(`source.desc.${s.id}`)}</div>
          </div>
          {#if s.supported}
            <Toggle bind:checked={enabled[s.id]} label={t(`source.${s.id}`)} disabled={!s.found && !enabled[s.id]} />
          {/if}
        </div>
      {/each}
      {#if sources.length && !anyFound}
        <div class="banner"><Icon name="info" size={16} />{t('onb.none')}</div>
      {/if}
    </section>

    {#if plans && providers.length}
      <section class="card">
        <h2>{t('onb.plans')}</h2>
        {#each providers as p (p)}
          <div class="list-row">
            <span class="grow">{plans.providers[p].label}</span>
            <Select label={plans.providers[p].label} bind:value={chosen[p]} minWidth={200} options={[{ value: '', label: t('onb.plan.none') }, ...plans.providers[p].plans.map((pl) => ({ value: pl.id, label: pl.name, sub: pl.relative }))]} />
          </div>
        {/each}
        <p class="subtle small note">{plans.notes[i18n.lang]}</p>
      </section>
    {/if}

    <p class="subtle small live"><Icon name="info" size={13} /> {t('onb.live')}</p>

    <section class="privacy">
      <button class="btn ghost" aria-expanded={showPrivacy} onclick={() => (showPrivacy = !showPrivacy)}><Icon name="lock" size={15} />{t('onb.privacy')}</button>
      {#if showPrivacy}<p class="subtle small">{t('settings.privacy.text')}</p>{/if}
    </section>

    <div class="cta">
      <button class="btn primary big" onclick={start} disabled={busy}>{t('onb.start')}</button>
    </div>
  </div>
</main>

<style>
  .onb {
    height: 100%;
    overflow-y: auto;
    display: flex;
    justify-content: center;
    padding: 48px 24px;
  }
  .panel {
    width: min(640px, 100%);
    /* bottom padding of a flex scroll container is not kept by WebView2; reserve it here */
    padding-bottom: 56px;
    height: max-content;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 10px;
    margin-bottom: 8px;
    animation: rise 500ms var(--ease) both;
  }
  .hero img {
    border-radius: 13px;
    box-shadow: var(--shadow);
  }
  .lead {
    max-width: 520px;
    font-size: 15px;
  }
  .lang {
    margin-top: 4px;
  }
  h2 {
    margin-bottom: 4px;
  }
  .state {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface-hover);
    color: var(--ink-3);
    flex: none;
  }
  .state.found {
    color: var(--good-ink);
  }
  .body,
  .grow {
    flex: 1;
    min-width: 0;
  }
  .banner {
    margin-top: 10px;
  }
  .note {
    margin-top: 8px;
  }
  .privacy p {
    margin-top: 6px;
    padding: 0 12px;
  }
  .live {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin: 4px 12px 10px;
    line-height: 1.5;
  }
  .live :global(svg) {
    flex: none;
    margin-top: 3px;
  }
  .cta {
    display: flex;
    justify-content: center;
    margin-top: 8px;
  }
  .big {
    height: 38px;
    padding: 0 28px;
    font-size: 14px;
    border-radius: 10px;
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
</style>
