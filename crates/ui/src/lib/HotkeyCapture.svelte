<script lang="ts">
  interface Props {
    label: string;
    value: string | null;
    onChange: (value: string | null) => void;
    disabled?: boolean;
  }

  let { label, value, onChange, disabled = false }: Props = $props();

  let recording = $state(false);

  // Render the saved binding in a slightly-friendlier form than the
  // raw `Ctrl+Shift+KeyG` string the Rust side stores. KeyA→A,
  // Digit1→1, leave function-key codes alone.
  function format(v: string | null): string {
    if (!v) return 'Not set';
    return v
      .split('+')
      .map((p) => {
        if (p.startsWith('Key') && p.length === 4) return p.slice(3);
        if (p.startsWith('Digit') && p.length === 6) return p.slice(5);
        return p;
      })
      .join(' + ');
  }

  function startRecording(): void {
    if (disabled) return;
    recording = true;
  }

  function clear(): void {
    if (disabled) return;
    recording = false;
    onChange(null);
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();

    if (e.key === 'Escape') {
      recording = false;
      return;
    }

    // Skip pure modifier presses — wait for the actual key.
    const modifierKeys = new Set(['Control', 'Shift', 'Alt', 'Meta']);
    if (modifierKeys.has(e.key)) {
      return;
    }

    const parts: string[] = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.shiftKey) parts.push('Shift');
    if (e.altKey) parts.push('Alt');
    if (e.metaKey) parts.push('Meta');
    // Require at least one modifier so a bare letter doesn't shadow
    // ordinary typing app-wide.
    if (parts.length === 0) {
      return;
    }
    parts.push(e.code);
    onChange(parts.join('+'));
    recording = false;
  }
</script>

<svelte:window onkeydown={handleKeyDown} />

<div class="flex flex-col gap-1 text-xs text-zinc-400">
  <span>{label}</span>
  <div class="flex items-center gap-2">
    <button
      type="button"
      onclick={startRecording}
      {disabled}
      class="flex-1 rounded bg-zinc-800 px-2 py-1 text-left text-sm text-zinc-100 disabled:opacity-50"
    >
      {#if recording}
        <span class="italic text-zinc-500">Press combo… (Esc to cancel)</span>
      {:else if value}
        <span class="font-mono">{format(value)}</span>
      {:else}
        <span class="text-zinc-500">Not set</span>
      {/if}
    </button>
    {#if value && !recording}
      <button
        type="button"
        onclick={clear}
        {disabled}
        class="rounded bg-zinc-800 px-2 py-1 text-xs text-zinc-400 hover:text-zinc-200 disabled:opacity-50"
      >
        Clear
      </button>
    {/if}
  </div>
</div>
