<script lang="ts">
  // The value has a fixed width so the bars of every row line up.
  let { pct, text, tone = 'accent' }: { pct: number; text: string; tone?: 'accent' | 'series' | 'critical' | 'teal' } = $props()
</script>

<span class="bv">
  <span class="bar" aria-hidden="true"><span class={tone} style="width:{Math.max(0, Math.min(100, pct))}%"></span></span>
  <span class="val">{text}</span>
</span>

<style>
  .bv {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    vertical-align: middle;
  }
  .bar {
    width: 48px;
    height: 5px;
    border-radius: 3px;
    background: var(--surface-hover);
    overflow: hidden;
    flex: none;
  }
  .bar span {
    display: block;
    height: 100%;
    border-radius: 3px;
  }
  .accent {
    background: var(--accent);
  }
  .series {
    background: var(--s1);
  }
  .critical {
    background: var(--critical);
  }
  .teal {
    background: var(--s3);
  }
  .val {
    min-width: 54px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
