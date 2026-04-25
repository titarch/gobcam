<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { listEmoji, syncStatus } from './lib/api';
  import type { EmojiInfo, SyncStatus } from './lib/emoji';
  import EmojiButton from './lib/EmojiButton.svelte';
  import { filterEmoji, groupEmoji } from './lib/search';

  let items = $state<readonly EmojiInfo[]>([]);
  let query = $state('');
  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;
  let sync = $state<SyncStatus | null>(null);
  let pollHandle: ReturnType<typeof setInterval> | null = null;
  let listError = $state<string | null>(null);

  let filtered = $derived(filterEmoji(items, query));
  let grouped = $derived(groupEmoji(filtered));

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

  onMount(async () => {
    await refreshEmoji();
    await pollSync();
    pollHandle = setInterval(pollSync, 1000);
  });

  onDestroy(() => {
    if (pollHandle) {
      clearInterval(pollHandle);
    }
    if (toastTimer) {
      clearTimeout(toastTimer);
    }
  });
</script>

<main class="flex h-screen flex-col bg-zinc-900 text-zinc-100">
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
      {#each grouped as section (section.group)}
        <section class="mb-4">
          <h2 class="mb-2 text-xs font-semibold uppercase tracking-wider text-zinc-500">
            {section.group}
          </h2>
          <div class="grid grid-cols-4 gap-2">
            {#each section.items as item (item.id)}
              <EmojiButton {item} onError={showError} />
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
