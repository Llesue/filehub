<script lang="ts">
  import { toast } from '../toast';
</script>

{#if $toast}
  <aside class="toast" role="status">{$toast}</aside>
{/if}

<style>
  .toast {
    position: fixed;
    right: 20px;
    bottom: 20px;
    z-index: 95;
    max-width: min(360px, calc(100vw - 40px));
    padding: 11px 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-2);
    font-size: 13.5px;
  }
</style>
