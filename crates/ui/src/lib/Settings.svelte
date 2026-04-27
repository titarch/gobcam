<script lang="ts">
  import { onMount } from 'svelte';
  import {
    type HotkeySettings,
    type InputDevice,
    type Mode,
    applySettings,
    currentHotkeys,
    currentSettings,
    listInputs,
    modeKey,
    modeLabel,
    quitApp,
    setColorScheme,
    setHotkeys,
  } from './api';
  import HotkeyCapture from './HotkeyCapture.svelte';

  interface Props {
    onError: (message: string) => void;
    previewEnabled: boolean;
    onPreviewChange: (enabled: boolean) => void;
    colorScheme: string;
    onColorSchemeChange: (scheme: string) => void;
  }

  let { onError, previewEnabled, onPreviewChange, colorScheme, onColorSchemeChange }: Props =
    $props();

  let inputs = $state<readonly InputDevice[]>([]);
  let selectedDevice = $state<string | null>(null);
  let selectedModeKey = $state<string | null>(null);
  let slotCount = $state(48);
  let slotDim = $state(256);
  let switching = $state(false);
  let open = $state(false);
  let hotkeys = $state<HotkeySettings>({ toggle: null, repeat: null, colorScheme: 'dark' });
  let savingHotkeys = $state(false);

  let currentDevice = $derived(
    inputs.find((d) => d.device === selectedDevice) ?? null,
  );
  let currentModes = $derived(currentDevice?.modes ?? []);

  function currentMode(): Mode | null {
    return currentDevice?.modes.find((m) => modeKey(m) === selectedModeKey) ?? null;
  }

  async function refresh(): Promise<void> {
    try {
      const [list, current, hk] = await Promise.all([
        listInputs(),
        currentSettings(),
        currentHotkeys(),
      ]);
      inputs = list;
      hotkeys = hk;
      slotCount = current.slot_count;
      slotDim = current.slot_dim;

      if (selectedDevice === null) {
        const persistedExists = list.some((d) => d.device === current.device);
        selectedDevice = persistedExists ? current.device : (list[0]?.device ?? null);
      }
      if (selectedModeKey === null && currentDevice) {
        const wantedKey = modeKey({
          width: current.width,
          height: current.height,
          fps_num: current.fps_num,
          fps_den: current.fps_den,
        });
        const exists = currentDevice.modes.some((m) => modeKey(m) === wantedKey);
        if (exists) {
          selectedModeKey = wantedKey;
        } else if (currentDevice.modes.length > 0) {
          const m = currentDevice.modes[0];
          if (m) {
            selectedModeKey = modeKey(m);
          }
        }
      }

      if (current.preview !== previewEnabled) {
        onPreviewChange(current.preview);
      }
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  async function commit(
    device: string,
    mode: Mode,
    preview: boolean,
    slot_count: number,
    slot_dim: number,
  ): Promise<void> {
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
        slot_count,
        slot_dim,
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
    void commit(device, mode, previewEnabled, slotCount, slotDim);
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
    void commit(currentDevice.device, mode, previewEnabled, slotCount, slotDim);
  }

  function togglePreview(enabled: boolean): void {
    onPreviewChange(enabled);
    const mode = currentMode();
    if (!currentDevice || !mode) {
      return;
    }
    void commit(currentDevice.device, mode, enabled, slotCount, slotDim);
  }

  function pickSlotCount(value: number): void {
    const next = Math.max(1, Math.min(128, Math.round(value)));
    slotCount = next;
    const mode = currentMode();
    if (!currentDevice || !mode) {
      return;
    }
    void commit(currentDevice.device, mode, previewEnabled, next, slotDim);
  }

  function pickSlotDim(value: number): void {
    slotDim = value;
    const mode = currentMode();
    if (!currentDevice || !mode) {
      return;
    }
    void commit(currentDevice.device, mode, previewEnabled, slotCount, value);
  }

  async function commitHotkeys(next: HotkeySettings): Promise<void> {
    if (savingHotkeys) {
      return;
    }
    const previous = hotkeys;
    hotkeys = next;
    savingHotkeys = true;
    try {
      await setHotkeys(next);
    } catch (e: unknown) {
      hotkeys = previous;
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      savingHotkeys = false;
    }
  }

  function setToggle(value: string | null): void {
    void commitHotkeys({ ...hotkeys, toggle: value });
  }

  function setRepeat(value: string | null): void {
    void commitHotkeys({ ...hotkeys, repeat: value });
  }

  async function changeColorScheme(scheme: string): Promise<void> {
    try {
      await setColorScheme(scheme);
      onColorSchemeChange(scheme);
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  async function handleQuit(): Promise<void> {
    try {
      await quitApp();
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
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
        <div class="relative">
          <select
            class="w-full appearance-none rounded bg-zinc-800 px-2 py-1 pr-7 text-sm text-zinc-100 focus:outline-none focus:ring-1 focus:ring-zinc-600 disabled:opacity-50"
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
          <span
            class="pointer-events-none absolute inset-y-0 right-2 flex items-center text-zinc-400"
            >▾</span
          >
        </div>
      </label>
      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        <span>Mode</span>
        <div class="relative">
          <select
            class="w-full appearance-none rounded bg-zinc-800 px-2 py-1 pr-7 text-sm text-zinc-100 focus:outline-none focus:ring-1 focus:ring-zinc-600 disabled:opacity-50"
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
          <span
            class="pointer-events-none absolute inset-y-0 right-2 flex items-center text-zinc-400"
            >▾</span
          >
        </div>
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
      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        <span class="flex items-center justify-between">
          <span>Reaction slots <span class="text-zinc-500">(restarts daemon)</span></span>
          <span class="font-mono text-zinc-300">{slotCount}</span>
        </span>
        <input
          type="range"
          min="4"
          max="96"
          step="1"
          value={slotCount}
          disabled={switching}
          oninput={(e) => {
            slotCount = Number((e.currentTarget as HTMLInputElement).value);
          }}
          onchange={(e) => pickSlotCount(Number((e.currentTarget as HTMLInputElement).value))}
          class="w-full"
        />
      </label>
      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        <span>Reaction quality <span class="text-zinc-500">(restarts daemon)</span></span>
        <div class="relative">
          <select
            class="w-full appearance-none rounded bg-zinc-800 px-2 py-1 pr-7 text-sm text-zinc-100 focus:outline-none focus:ring-1 focus:ring-zinc-600 disabled:opacity-50"
            disabled={switching}
            value={String(slotDim)}
            onchange={(e) => pickSlotDim(Number((e.currentTarget as HTMLSelectElement).value))}
          >
            <option value="256">High (256 px)</option>
            <option value="192">Medium (192 px)</option>
            <option value="128">Low (128 px)</option>
          </select>
          <span
            class="pointer-events-none absolute inset-y-0 right-2 flex items-center text-zinc-400"
            >▾</span
          >
        </div>
      </label>
      {#if switching}
        <p class="text-xs text-zinc-500">Applying — daemon restarting…</p>
      {/if}

      <div class="mt-2 flex flex-col gap-2 border-t border-zinc-800 pt-3">
        <span class="text-xs uppercase tracking-wider text-zinc-500">Appearance</span>
        <label class="flex flex-col gap-1 text-xs text-zinc-400">
          <span>Color scheme</span>
          <div class="relative">
            <select
              class="w-full appearance-none rounded bg-zinc-800 px-2 py-1 pr-7 text-sm text-zinc-100 focus:outline-none focus:ring-1 focus:ring-zinc-600"
              value={colorScheme}
              onchange={(e) => changeColorScheme((e.currentTarget as HTMLSelectElement).value)}
            >
              <option value="dark">Dark (always)</option>
              <option value="light dark">Follow system</option>
            </select>
            <span
              class="pointer-events-none absolute inset-y-0 right-2 flex items-center text-zinc-400"
              >▾</span
            >
          </div>
        </label>
      </div>

      <div class="mt-2 flex flex-col gap-2 border-t border-zinc-800 pt-3">
        <span class="text-xs uppercase tracking-wider text-zinc-500">Hotkeys</span>
        <HotkeyCapture
          label="Show / hide panel"
          value={hotkeys.toggle}
          onChange={setToggle}
          disabled={savingHotkeys}
        />
        <HotkeyCapture
          label="Trigger most recent emoji"
          value={hotkeys.repeat}
          onChange={setRepeat}
          disabled={savingHotkeys}
        />
        <p class="text-[10px] text-zinc-600">
          Hotkeys work app-wide (window hidden too). The X button hides to tray;
          Quit fully exits.
        </p>
      </div>

      <div class="mt-2 flex justify-end border-t border-zinc-800 pt-3">
        <button
          type="button"
          onclick={handleQuit}
          class="rounded bg-zinc-800 px-3 py-1 text-xs text-zinc-300 hover:bg-zinc-700"
        >
          Quit Gobcam
        </button>
      </div>
    </div>
  {/if}
</div>
