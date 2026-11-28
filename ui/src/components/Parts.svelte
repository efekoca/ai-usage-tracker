<script lang="ts">
  let { items, title }: { items: (string | null | undefined | false)[]; title?: string } = $props()
  const shown = $derived(items.filter((x): x is string => !!x))
</script>

<span class="spaced" {title}>{#each shown as p, i}{#if i > 0}{' '}{/if}<span>{p}</span>{/each}</span>
