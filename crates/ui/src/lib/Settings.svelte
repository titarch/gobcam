<script lang="ts">
  import { onMount } from 'svelte';
  import {
    type InputDevice,
    type Mode,
    applySettings,
    listInputs,
    modeKey,
    modeLabel,
  } from './api';

  interface Props {
    onError: (message: string) => void;
    previewEnabled: boolean;
    onPreviewChange: (enabled: boolean) => void;
  }

  let { onError, previewEnabled, onPreviewChange }: Props = $props();

  let inputs = $state<readonly InputDevice[]>([]);
  let selectedDevice = $state<string | null>(null);
  let selectedModeKey = $state<string | null>(null);
  let switching = $state(false);
  let open = $state(false);

  let currentDevice = $derived(
    inputs.find((d) => d.device === selectedDevice) ?? null,
  );
  let currentModes = $derived(currentDevice?.modes ?? []);

  function currentMode(): Mode | null {
    return currentDevice?.modes.find((m) => modeKey(m) === selectedModeKey) ?? null;
  }

  async function refresh(): Promise<void> {
    try {
      const list = await listInputs();
      inputs = list;
      if (selectedDevice === null && list.length > 0) {
        selectedDevice = list[0]?.device ?? null;
      }
      if (selectedModeKey === null && currentDevice && currentDevice.modes.length > 0) {
        const m = currentDevice.modes[0];
        if (m) {
          selectedModeKey = modeKey(m);
        }
      }
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  async function commit(device: string, mode: Mode, preview: boolean): Promise<void> {
    if (switching) {
      return;
    }
    switching = true;
    try {
      await applySettings({
        device,
        width: mode.width,
        height: mode.height,
        fps_num: mode.fps_num,
        fps_den: mode.fps_den,
        preview,
      });
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      switching = false;
    }
  }

  function pickDevice(device: string): void {
    selectedDevice = device;
    const dev = inputs.find((d) => d.device === device);
    const mode = dev?.modes[0];
    if (!mode) {
      return;
    }
    selectedModeKey = modeKey(mode);
    void commit(device, mode, previewEnabled);
  }

  function pickMode(key: string): void {
    selectedModeKey = key;
    if (!currentDevice) {
      return;
    }
    const mode = currentDevice.modes.find((m) => modeKey(m) === key);
    if (!mode) {
      return;
    }
    void commit(currentDevice.device, mode, previewEnabled);
  }

  function togglePreview(enabled: boolean): void {
    onPreviewChange(enabled);
    const mode = currentMode();
    if (!currentDevice || !mode) {
      return;
    }
    void commit(currentDevice.device, mode, enabled);
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
          value={selectedDevice ?? ''}
          onchange={(e) => pickDevice((e.currentTarget as HTMLSelectElement).value)}
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
      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        <span>Mode</span>
        <select
          class="rounded bg-zinc-800 px-2 py-1 text-sm text-zinc-100 focus:outline-none focus:ring-1 focus:ring-zinc-600 disabled:opacity-50"
          disabled={switching || currentModes.length === 0}
          value={selectedModeKey ?? ''}
          onchange={(e) => pickMode((e.currentTarget as HTMLSelectElement).value)}
        >
          {#if currentModes.length === 0}
            <option value="">(no modes reported)</option>
          {:else}
            {#each currentModes as mode (modeKey(mode))}
              <option value={modeKey(mode)}>{modeLabel(mode)}</option>
            {/each}
          {/if}
        </select>
      </label>
      <label class="flex items-center gap-2 text-xs text-zinc-300">
        <input
          type="checkbox"
          checked={previewEnabled}
          disabled={switching}
          onchange={(e) => togglePreview((e.currentTarget as HTMLInputElement).checked)}
          class="h-3 w-3 rounded"
        />
        <span>Live preview <span class="text-zinc-500">(restarts daemon)</span></span>
      </label>
      {#if switching}
        <p class="text-xs text-zinc-500">Applying — daemon restarting…</p>
      {/if}
    </div>
  {/if}
</div>
