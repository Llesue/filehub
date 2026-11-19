<script lang="ts">
  import { splitFileName } from '../filename';

  let { name, isDir = false }: { name: string; isDir?: boolean } = $props();
  let parts = $derived(isDir ? null : splitFileName(name));
</script>

<span class="file-name" title={name}>
  {#if parts}
    <span class="name-stem">{parts.stem}</span><span class="name-ext">{parts.ext}</span>
  {:else}
    <span class="name-stem">{name}</span>
  {/if}
</span>

<style>
  .file-name {
    display: inline-flex;
    overflow: hidden;
    max-width: 100%;
    min-width: 0;
    align-items: baseline;
    white-space: nowrap;
  }

  .name-stem {
    flex: 0 1 auto;
    overflow: hidden;
    min-width: 0;
    text-overflow: ellipsis;
  }

  .name-ext {
    flex: 0 0 auto;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
