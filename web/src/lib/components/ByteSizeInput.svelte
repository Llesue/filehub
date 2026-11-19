<script lang="ts">
  let {
    value,
    min = 0,
    max = Number.MAX_SAFE_INTEGER,
    onSave,
  }: {
    value: number;
    min?: number;
    max?: number;
    onSave: (bytes: number) => void;
  } = $props();

  const UNITS = [
    { label: 'B', factor: 1 },
    { label: 'KB', factor: 1024 },
    { label: 'MB', factor: 1024 ** 2 },
    { label: 'GB', factor: 1024 ** 3 },
    { label: 'TB', factor: 1024 ** 4 },
  ];

  let number = $state(0);
  let unit = $state(1);
  let editing = $state(false);
  let error = $state('');

  function pick(bytes: number): { number: number; unit: number } {
    let index = 0;
    for (let i = UNITS.length - 1; i >= 0; i -= 1) {
      const candidate = UNITS[i];
      if (candidate && bytes >= candidate.factor) {
        index = i;
        break;
      }
    }
    const selected = UNITS[index] ?? UNITS[0]!;
    return { number: bytes / selected.factor, unit: index };
  }

  $effect(() => {
    if (editing) return;
    const next = pick(value);
    number = Number(next.number.toFixed(next.number >= 100 ? 0 : 2));
    unit = next.unit;
  });

  function commit(): void {
    editing = false;
    const selected = UNITS[unit] ?? UNITS[0]!;
    const bytes = Math.round(Number(number) * selected.factor);
    if (!Number.isFinite(bytes) || bytes < min || bytes > max) {
      error = `范围 ${min} – ${max} 字节`;
      const next = pick(value);
      number = Number(next.number.toFixed(2));
      unit = next.unit;
      return;
    }
    error = '';
    if (bytes !== value) onSave(bytes);
  }
</script>

<div class="byte-input">
  <input
    bind:value={number}
    type="number"
    min="0"
    step="any"
    onfocus={() => (editing = true)}
    onblur={commit}
    onkeydown={(event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        event.currentTarget.blur();
      }
    }}
  />
  <select bind:value={unit} aria-label="unit">
    {#each UNITS as item, index (item.label)}
      <option value={index}>{item.label}</option>
    {/each}
  </select>
  {#if error}<small class="byte-error">{error}</small>{/if}
</div>

<style>
  .byte-input {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .byte-input input {
    width: 110px;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    text-align: right;
  }

  .byte-input select {
    padding: 6px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  .byte-error {
    color: var(--danger);
    font-size: 12px;
  }
</style>
