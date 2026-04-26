<script lang="ts">
  import { onMount } from 'svelte';
  import { type InputDevice, listInputs, switchInput } from './api';

  interface Props {
    onError: (message: string) => void;
  }

  let { onError }: Props = $props();

  let inputs = $state<readonly InputDevice[]>([]);
  let selected = $state<string | null>(null);
  let switching = $state(false);
  let open = $state(false);

  async function refresh(): Promise<void> {
    try {
      const list = await listInputs();
      inputs = list;
      // The daemon doesn't tell us which input *it* is using; default
      // the dropdown to the first option until the user picks one.
      if (selected === null && list.length > 0) {
        selected = list[0]?.device ?? null;
      }
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  async function pick(device: string): Promise<void> {
    if (device === selected || switching) {
      return;
    }
    switching = true;
    selected = device;
    try {
      await switchInput(device);
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      switching = false;
    }
  }

  onMount(() => {
    void refresh();
  });
</script>

<div class="border-b border-zinc-800 bg-zinc-900/80">
  <button
    type="button"
    onclick={() => {
      open = !open;
    }}
    class="flex w-full items-center justify-between px-3 py-2 text-xs uppercase tracking-wider text-zinc-500 hover:text-zinc-300"
  >
    <span>Settings</span>
    <span class="font-mono">{open ? '▾' : '▸'}</span>
  </button>
  {#if open}
    <div class="flex flex-col gap-2 px-3 pb-3">
      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        <span>Webcam</span>
        <select
          class="rounded bg-zinc-800 px-2 py-1 text-sm text-zinc-100 focus:outline-none focus:ring-1 focus:ring-zinc-600 disabled:opacity-50"
          disabled={switching || inputs.length === 0}
          value={selected ?? ''}
          onchange={(e) => pick((e.currentTarget as HTMLSelectElement).value)}
        >
          {#if inputs.length === 0}
            <option value="">(none detected)</option>
          {:else}
            {#each inputs as input (input.device)}
              <option value={input.device}>{input.name} — {input.device}</option>
            {/each}
          {/if}
        </select>
      </label>
      {#if switching}
        <p class="text-xs text-zinc-500">Switching webcam — daemon restarting…</p>
      {/if}
    </div>
  {/if}
</div>
