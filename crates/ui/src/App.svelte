<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    listEmoji,
    listRecents,
    setupStatus,
    syncStatus,
    type SetupStatus,
  } from './lib/api';
  import type { EmojiInfo, SyncStatus } from './lib/emoji';
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

  let filtered = $derived(filterEmoji(items, query));
  let grouped = $derived(groupEmoji(filtered));
  // O(n) over the catalog rebuilt only when items changes — cheap.
  let byId = $derived(new Map(items.map((it) => [it.id, it])));
  // Filter out any recents that aren't in the current catalog (an
  // emoji removed from upstream after the user triggered it).
  let recentItems = $derived(
    recents.map((id) => byId.get(id)).filter((x): x is EmojiInfo => x !== undefined),
  );

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
      // setup_status is infallible from the daemon's perspective —
      // a failure here is a Tauri/IPC issue, not a setup-required
      // signal. Leave the previous value in place.
    }
  }

  async function startMainLoops(): Promise<void> {
    await refreshEmoji();
    await refreshRecents();
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
    // Hotkey-triggered fires don't go through the JS layer, so when
    // the window comes back into view from the tray pull a fresh
    // recents list to surface them.
    if (document.visibilityState === 'visible') {
      void refreshRecents();
    }
  }

  onMount(async () => {
    await refreshSetupStatus();
    if (setup?.required) {
      // Don't start the daemon-dependent polls until setup finishes.
      return;
    }
    document.addEventListener('visibilitychange', handleVisibilityChange);
    await startMainLoops();
  });

  onDestroy(() => {
    document.removeEventListener('visibilitychange', handleVisibilityChange);
    if (pollHandle) {
      clearInterval(pollHandle);
    }
    if (toastTimer) {
      clearTimeout(toastTimer);
    }
  });
</script>

{#if setup?.required}
  <Setup status={setup} onComplete={handleSetupComplete} />
{:else}
<main class="flex h-screen flex-col bg-zinc-900 text-zinc-100">
  <Settings
    onError={showError}
    {previewEnabled}
    onPreviewChange={(enabled) => {
      previewEnabled = enabled;
    }}
  />
  <Preview enabled={previewEnabled} />
  <header class="flex flex-col gap-2 border-b border-zinc-800 p-3">
    <input
      type="search"
      bind:value={query}
      placeholder="Search emoji…"
      class="w-full rounded bg-zinc-800 px-3 py-2 text-sm placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-600"
    />
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
      {#if query.trim() === '' && recentItems.length > 0}
        <section class="mb-4">
          <h2 class="mb-2 text-xs font-semibold uppercase tracking-wider text-zinc-500">
            Recent
          </h2>
          <div class="grid grid-cols-4 gap-2">
            {#each recentItems as item (item.id)}
              <EmojiButton {item} onError={showError} onTriggered={refreshRecents} />
            {/each}
          </div>
        </section>
      {/if}
      {#each grouped as section (section.group)}
        <section class="mb-4">
          <h2 class="mb-2 text-xs font-semibold uppercase tracking-wider text-zinc-500">
            {section.group}
          </h2>
          <div class="grid grid-cols-4 gap-2">
            {#each section.items as item (item.id)}
              <EmojiButton {item} onError={showError} onTriggered={refreshRecents} />
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
