<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import {
    currentHotkeys,
    listEmoji,
    listFavorites,
    listRecents,
    setupStatus,
    syncStatus,
    toggleFavorite,
    type SetupStatus,
  } from './lib/api';
  import type { EmojiInfo, SyncStatus } from './lib/emoji';
  import Animations from './lib/Animations.svelte';
  import EmojiButton from './lib/EmojiButton.svelte';
  import Preview from './lib/Preview.svelte';
  import Settings from './lib/Settings.svelte';
  import Setup from './lib/Setup.svelte';
  import { filterEmoji, groupEmoji } from './lib/search';

  let items = $state<readonly EmojiInfo[]>([]);
  let query = $state('');
  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;
  let sync = $state<SyncStatus | null>(null);
  let pollHandle: ReturnType<typeof setInterval> | null = null;
  let listError = $state<string | null>(null);
  let previewEnabled = $state(false);
  let setup = $state<SetupStatus | null>(null);
  let recents = $state<readonly string[]>([]);
  let favorites = $state<readonly string[]>([]);
  let colorScheme = $state('dark');
  let safeMode = $state(false);
  let view = $state<'main' | 'animations'>('main');

  let visibleItems = $derived(
    safeMode ? items.filter((it) => !it.is_safe_mode_excluded) : items,
  );
  let filtered = $derived(filterEmoji(visibleItems, query));
  let grouped = $derived(groupEmoji(filtered));
  let byId = $derived(new Map(visibleItems.map((it) => [it.id, it])));
  let recentItems = $derived(
    recents.map((id) => byId.get(id)).filter((x): x is EmojiInfo => x !== undefined),
  );
  let favoriteItems = $derived(
    favorites.map((id) => byId.get(id)).filter((x): x is EmojiInfo => x !== undefined),
  );
  let favSet = $derived(new Set(favorites));

  function applyColorScheme(scheme: string): void {
    document.documentElement.style.colorScheme = scheme;
  }

  function showError(message: string): void {
    toast = message;
    if (toastTimer) {
      clearTimeout(toastTimer);
    }
    toastTimer = setTimeout(() => {
      toast = null;
    }, 3500);
  }

  async function refreshEmoji(): Promise<void> {
    try {
      const fetched = await listEmoji();
      items = fetched;
      listError = null;
    } catch (e: unknown) {
      listError = e instanceof Error ? e.message : String(e);
    }
  }

  async function refreshRecents(): Promise<void> {
    try {
      recents = await listRecents();
    } catch {
      // Daemon may be transitioning; leave the previous list visible.
    }
  }

  async function refreshFavorites(): Promise<void> {
    try {
      favorites = await listFavorites();
    } catch {
      // non-fatal
    }
  }

  async function handleFavoriteToggle(id: string): Promise<void> {
    try {
      await toggleFavorite(id);
      await refreshFavorites();
    } catch (e: unknown) {
      showError(e instanceof Error ? e.message : String(e));
    }
  }

  async function pollSync(): Promise<void> {
    try {
      const status = await syncStatus();
      sync = status;
      if (status.complete && pollHandle) {
        clearInterval(pollHandle);
        pollHandle = null;
      }
    } catch {
      // Daemon may not be up yet; keep polling.
    }
  }

  async function refreshSetupStatus(): Promise<void> {
    try {
      setup = await setupStatus();
    } catch {
      // infallible from daemon's perspective; leave previous value.
    }
  }

  async function loadColorScheme(): Promise<void> {
    try {
      const hk = await currentHotkeys();
      colorScheme = hk.colorScheme;
      safeMode = hk.safeMode;
      applyColorScheme(hk.colorScheme);
    } catch {
      // non-fatal; default "dark" stays
    }
  }

  async function startMainLoops(): Promise<void> {
    await Promise.all([refreshEmoji(), refreshRecents(), refreshFavorites(), loadColorScheme()]);
    await pollSync();
    if (!pollHandle) {
      pollHandle = setInterval(pollSync, 1000);
    }
  }

  async function handleSetupComplete(): Promise<void> {
    await refreshSetupStatus();
    if (setup && !setup.required) {
      await startMainLoops();
    }
  }

  function handleVisibilityChange(): void {
    if (document.visibilityState === 'visible') {
      void refreshRecents();
    }
  }

  let unlistenSafeMode: UnlistenFn | null = null;

  async function subscribeSafeModeBlocked(): Promise<void> {
    try {
      unlistenSafeMode = await listen<string>('safe-mode-blocked-trigger', () => {
        showError('Hidden by safe mode — turn it off in Settings to use that emoji.');
      });
    } catch (e: unknown) {
      // Non-fatal: event API may be denied by capabilities. The toast
      // is a nice-to-have on the hotkey-suppressed path.
      console.warn('safe-mode listen failed', e);
    }
  }

  onMount(async () => {
    await refreshSetupStatus();
    if (setup?.required) {
      return;
    }
    document.addEventListener('visibilitychange', handleVisibilityChange);
    await startMainLoops();
    void subscribeSafeModeBlocked();
  });

  onDestroy(() => {
    document.removeEventListener('visibilitychange', handleVisibilityChange);
    if (pollHandle) {
      clearInterval(pollHandle);
    }
    if (toastTimer) {
      clearTimeout(toastTimer);
    }
    if (unlistenSafeMode) {
      unlistenSafeMode();
    }
  });
</script>

{#if setup?.required}
  <Setup status={setup} onComplete={handleSetupComplete} />
{:else if view === 'animations'}
  <Animations
    onError={showError}
    onClose={() => {
      view = 'main';
    }}
  />
{:else}
<main class="flex h-screen flex-col bg-zinc-900 text-zinc-100">
  <Settings
    onError={showError}
    {previewEnabled}
    onPreviewChange={(enabled) => {
      previewEnabled = enabled;
    }}
    {colorScheme}
    onColorSchemeChange={(scheme) => {
      colorScheme = scheme;
      applyColorScheme(scheme);
    }}
    {safeMode}
    onSafeModeChange={(enabled) => {
      safeMode = enabled;
    }}
  />
  <Preview enabled={previewEnabled} />
  <header class="flex flex-col gap-2 border-b border-zinc-800 p-3">
    <div class="flex items-center gap-2">
      <input
        type="search"
        bind:value={query}
        placeholder="Search emoji…"
        class="flex-1 rounded bg-zinc-800 px-3 py-2 text-sm placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-600"
      />
      <button
        type="button"
        onclick={() => {
          view = 'animations';
        }}
        class="rounded bg-zinc-800 px-2 py-2 text-sm hover:bg-zinc-700"
        aria-label="Animation settings"
        title="Animation settings"
      >
        ✨
      </button>
    </div>
    {#if sync && !sync.complete && sync.total > 0}
      <div class="flex items-center gap-2 text-xs text-zinc-400">
        <div class="h-1 flex-1 overflow-hidden rounded bg-zinc-800">
          <div
            class="h-full bg-zinc-400 transition-[width]"
            style="width: {(sync.fetched / sync.total) * 100}%"
          ></div>
        </div>
        <span>{sync.fetched}/{sync.total}</span>
      </div>
    {/if}
  </header>

  <div class="flex-1 overflow-y-auto p-3">
    {#if listError}
      <div class="rounded bg-red-900/60 p-2 text-sm" role="alert">
        Daemon offline: {listError}
      </div>
    {:else if items.length === 0}
      <div class="text-center text-sm text-zinc-500">Loading catalog…</div>
    {:else if filtered.length === 0}
      <div class="text-center text-sm text-zinc-500">No matches.</div>
    {:else}
      {#if query.trim() === '' && favoriteItems.length > 0}
        <section class="mb-4">
          <h2 class="mb-2 text-xs font-semibold uppercase tracking-wider text-zinc-500">
            Favorites
          </h2>
          <div class="grid grid-cols-[repeat(auto-fill,minmax(72px,1fr))] gap-2">
            {#each favoriteItems as item (item.id)}
              <EmojiButton
                {item}
                onError={showError}
                onTriggered={refreshRecents}
                isFavorite={true}
                onFavoriteToggle={handleFavoriteToggle}
              />
            {/each}
          </div>
        </section>
      {/if}
      {#if query.trim() === '' && recentItems.length > 0}
        <section class="mb-4">
          <h2 class="mb-2 text-xs font-semibold uppercase tracking-wider text-zinc-500">
            Recent
          </h2>
          <div class="grid grid-cols-[repeat(auto-fill,minmax(72px,1fr))] gap-2">
            {#each recentItems as item (item.id)}
              <EmojiButton
                {item}
                onError={showError}
                onTriggered={refreshRecents}
                isFavorite={favSet.has(item.id)}
                onFavoriteToggle={handleFavoriteToggle}
              />
            {/each}
          </div>
        </section>
      {/if}
      {#each grouped as section (section.group)}
        <section class="mb-4">
          <h2 class="mb-2 text-xs font-semibold uppercase tracking-wider text-zinc-500">
            {section.group}
          </h2>
          <div class="grid grid-cols-[repeat(auto-fill,minmax(72px,1fr))] gap-2">
            {#each section.items as item (item.id)}
              <EmojiButton
                {item}
                onError={showError}
                onTriggered={refreshRecents}
                isFavorite={favSet.has(item.id)}
                onFavoriteToggle={handleFavoriteToggle}
              />
            {/each}
          </div>
        </section>
      {/each}
    {/if}
  </div>

  {#if toast}
    <div class="border-t border-zinc-800 bg-red-900/60 p-2 text-sm" role="alert">
      {toast}
    </div>
  {/if}
</main>
{/if}
