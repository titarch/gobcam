<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { type AnimationConfig, currentAnimations, setAnimationConfig } from './api';

  interface Props {
    onError: (message: string) => void;
    onClose: () => void;
  }

  let { onError, onClose }: Props = $props();

  const DEFAULTS: AnimationConfig = {
    lifetime_ms: 5000,
    fade_in_ms: 200,
    fade_out_start_ms: 3000,
    fade_out_ms: 2000,
    travel_px: 480,
    speed_jitter_pct: 0.25,
    start_x_fraction: 0.5,
    start_y_offset_px: 80,
    x_jitter_px: 220,
    direction_angle_deg: 90,
    apng_speed_multiplier: 1,
    max_concurrent: 32,
    drop_policy: 'drop_new',
    overrides: {},
  };

  let cfg = $state<AnimationConfig>({ ...DEFAULTS });
  let loaded = $state(false);
  let saving = $state(false);
  let pending: ReturnType<typeof setTimeout> | null = null;

  async function refresh(): Promise<void> {
    try {
      const fresh = await currentAnimations();
      cfg = { ...fresh };
      loaded = true;
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  // Debounce ~150 ms after the last keystroke before pushing to the
  // daemon — slider drags would otherwise fire dozens of IPC commands.
  function scheduleCommit(): void {
    if (pending) {
      clearTimeout(pending);
    }
    pending = setTimeout(() => {
      pending = null;
      void commit();
    }, 150);
  }

  async function commit(): Promise<void> {
    if (saving) {
      return;
    }
    saving = true;
    try {
      await setAnimationConfig({ ...cfg });
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  function num(value: string, fallback: number): number {
    const n = Number(value);
    return Number.isFinite(n) ? n : fallback;
  }

  function update<K extends keyof AnimationConfig>(key: K, value: AnimationConfig[K]): void {
    cfg = { ...cfg, [key]: value };
    scheduleCommit();
  }

  function reset(): void {
    cfg = { ...DEFAULTS };
    scheduleCommit();
  }

  onMount(() => {
    void refresh();
  });

  onDestroy(() => {
    if (pending) {
      clearTimeout(pending);
      void commit();
    }
  });
</script>

<main class="flex h-screen flex-col bg-zinc-900 text-zinc-100">
  <header class="flex items-center justify-between border-b border-zinc-800 px-3 py-2">
    <button
      type="button"
      onclick={onClose}
      class="rounded px-2 py-1 text-xs text-zinc-300 hover:bg-zinc-800"
      aria-label="Back"
    >
      ← Back
    </button>
    <h1 class="text-sm font-semibold">Animations</h1>
    <button
      type="button"
      onclick={reset}
      class="rounded bg-zinc-800 px-2 py-1 text-xs text-zinc-200 hover:bg-zinc-700"
    >
      Reset
    </button>
  </header>

  <form class="flex-1 overflow-y-auto p-3 text-xs text-zinc-300">
    {#if !loaded}
      <p class="text-center text-zinc-500">Loading…</p>
    {:else}
      <fieldset class="mb-4 flex flex-col gap-2 rounded border border-zinc-800 p-3">
        <legend class="px-1 text-[10px] uppercase tracking-wider text-zinc-500">Position</legend>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Spawn x (% of width)</span>
            <span class="font-mono text-zinc-400">{(cfg.start_x_fraction * 100).toFixed(0)}%</span>
          </span>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            value={cfg.start_x_fraction}
            oninput={(e) =>
              update('start_x_fraction', num((e.currentTarget as HTMLInputElement).value, 0.5))}
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Horizontal jitter (px)</span>
            <span class="font-mono text-zinc-400">±{cfg.x_jitter_px.toFixed(0)}</span>
          </span>
          <input
            type="range"
            min="0"
            max="600"
            step="5"
            value={cfg.x_jitter_px}
            oninput={(e) =>
              update('x_jitter_px', num((e.currentTarget as HTMLInputElement).value, 220))}
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Spawn offset above bottom (px)</span>
            <span class="font-mono text-zinc-400">{cfg.start_y_offset_px.toFixed(0)}</span>
          </span>
          <input
            type="range"
            min="0"
            max="400"
            step="5"
            value={cfg.start_y_offset_px}
            oninput={(e) =>
              update('start_y_offset_px', num((e.currentTarget as HTMLInputElement).value, 80))}
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Travel distance (px)</span>
            <span class="font-mono text-zinc-400">{cfg.travel_px.toFixed(0)}</span>
          </span>
          <input
            type="range"
            min="0"
            max="1080"
            step="10"
            value={cfg.travel_px}
            oninput={(e) =>
              update('travel_px', num((e.currentTarget as HTMLInputElement).value, 480))}
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Direction angle (°, 90 = up)</span>
            <span class="font-mono text-zinc-400">{cfg.direction_angle_deg.toFixed(0)}°</span>
          </span>
          <input
            type="range"
            min="0"
            max="180"
            step="1"
            value={cfg.direction_angle_deg}
            oninput={(e) =>
              update(
                'direction_angle_deg',
                num((e.currentTarget as HTMLInputElement).value, 90),
              )}
          />
        </label>
      </fieldset>

      <fieldset class="mb-4 flex flex-col gap-2 rounded border border-zinc-800 p-3">
        <legend class="px-1 text-[10px] uppercase tracking-wider text-zinc-500">Timing</legend>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Lifetime (ms)</span>
            <span class="font-mono text-zinc-400">{cfg.lifetime_ms}</span>
          </span>
          <input
            type="range"
            min="500"
            max="15000"
            step="100"
            value={cfg.lifetime_ms}
            oninput={(e) =>
              update(
                'lifetime_ms',
                Math.round(num((e.currentTarget as HTMLInputElement).value, 5000)),
              )}
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Fade in (ms)</span>
            <span class="font-mono text-zinc-400">{cfg.fade_in_ms}</span>
          </span>
          <input
            type="range"
            min="0"
            max="2000"
            step="50"
            value={cfg.fade_in_ms}
            oninput={(e) =>
              update(
                'fade_in_ms',
                Math.round(num((e.currentTarget as HTMLInputElement).value, 200)),
              )}
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Fade-out start (ms)</span>
            <span class="font-mono text-zinc-400">{cfg.fade_out_start_ms}</span>
          </span>
          <input
            type="range"
            min="0"
            max="15000"
            step="100"
            value={cfg.fade_out_start_ms}
            oninput={(e) =>
              update(
                'fade_out_start_ms',
                Math.round(num((e.currentTarget as HTMLInputElement).value, 3000)),
              )}
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Fade-out duration (ms)</span>
            <span class="font-mono text-zinc-400">{cfg.fade_out_ms}</span>
          </span>
          <input
            type="range"
            min="0"
            max="5000"
            step="50"
            value={cfg.fade_out_ms}
            oninput={(e) =>
              update(
                'fade_out_ms',
                Math.round(num((e.currentTarget as HTMLInputElement).value, 2000)),
              )}
          />
        </label>
      </fieldset>

      <fieldset class="mb-4 flex flex-col gap-2 rounded border border-zinc-800 p-3">
        <legend class="px-1 text-[10px] uppercase tracking-wider text-zinc-500">Speed</legend>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Speed jitter</span>
            <span class="font-mono text-zinc-400">±{(cfg.speed_jitter_pct * 100).toFixed(0)}%</span>
          </span>
          <input
            type="range"
            min="0"
            max="0.75"
            step="0.01"
            value={cfg.speed_jitter_pct}
            oninput={(e) =>
              update(
                'speed_jitter_pct',
                num((e.currentTarget as HTMLInputElement).value, 0.25),
              )}
          />
        </label>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>APNG playback speed ×</span>
            <span class="font-mono text-zinc-400">{cfg.apng_speed_multiplier.toFixed(2)}</span>
          </span>
          <input
            type="range"
            min="0.1"
            max="5"
            step="0.05"
            value={cfg.apng_speed_multiplier}
            oninput={(e) =>
              update(
                'apng_speed_multiplier',
                num((e.currentTarget as HTMLInputElement).value, 1),
              )}
          />
        </label>
      </fieldset>

      <fieldset class="mb-4 flex flex-col gap-2 rounded border border-zinc-800 p-3">
        <legend class="px-1 text-[10px] uppercase tracking-wider text-zinc-500">Limits</legend>

        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between">
            <span>Max concurrent reactions</span>
            <span class="font-mono text-zinc-400">{cfg.max_concurrent}</span>
          </span>
          <input
            type="range"
            min="1"
            max="96"
            step="1"
            value={cfg.max_concurrent}
            oninput={(e) =>
              update(
                'max_concurrent',
                Math.round(num((e.currentTarget as HTMLInputElement).value, 32)),
              )}
          />
          <span class="text-[10px] text-zinc-500">
            Capped at the daemon's slot count (Settings → Reaction slots).
          </span>
        </label>

        <label class="flex flex-col gap-1">
          <span>Drop policy</span>
          <div class="relative">
            <select
              class="w-full appearance-none rounded bg-zinc-800 px-2 py-1 pr-7 text-sm text-zinc-100 focus:outline-none focus:ring-1 focus:ring-zinc-600"
              value={cfg.drop_policy}
              onchange={(e) => {
                const v = (e.currentTarget as HTMLSelectElement).value;
                if (v === 'drop_new' || v === 'drop_oldest') {
                  update('drop_policy', v);
                }
              }}
            >
              <option value="drop_new">Drop new (silently ignore)</option>
              <option value="drop_oldest">Drop oldest (recycle slot)</option>
            </select>
            <span
              class="pointer-events-none absolute inset-y-0 right-2 flex items-center text-zinc-400"
              >▾</span
            >
          </div>
        </label>
      </fieldset>
    {/if}
  </form>
</main>
