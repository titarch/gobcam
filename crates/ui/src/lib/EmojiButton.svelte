<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { trigger } from './api';
  import type { EmojiInfo } from './emoji';

  interface Props {
    item: EmojiInfo;
    onError: (message: string) => void;
    onTriggered?: (id: string) => void;
    isFavorite?: boolean;
    onFavoriteToggle?: (id: string) => void;
  }

  let { item, onError, onTriggered, isFavorite = false, onFavoriteToggle }: Props = $props();
  let busy = $state(false);
  let imageOk = $state(true);

  let src = $derived(convertFileSrc(item.preview_path));

  async function handleClick(): Promise<void> {
    busy = true;
    try {
      await trigger(item.id);
      onTriggered?.(item.id);
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  function handleFavoriteClick(e: MouseEvent): void {
    e.stopPropagation();
    onFavoriteToggle?.(item.id);
  }
</script>

<div class="group relative aspect-square">
  <button
    type="button"
    onclick={handleClick}
    disabled={busy}
    title={item.name}
    aria-label={item.name}
    class="flex h-full w-full items-center justify-center rounded-lg bg-zinc-800 shadow transition hover:bg-zinc-700 active:scale-95 disabled:opacity-50"
  >
    {#if imageOk}
      <img
        src={src}
        alt={item.glyph}
        class="h-12 w-12 object-contain"
        loading="lazy"
        decoding="async"
        onerror={() => {
          imageOk = false;
        }}
      />
    {:else}
      <span class="text-3xl leading-none">{item.glyph}</span>
    {/if}
  </button>
  {#if onFavoriteToggle !== undefined}
    <button
      type="button"
      onclick={handleFavoriteClick}
      aria-label={isFavorite ? 'Remove from favorites' : 'Add to favorites'}
      class="absolute right-0.5 top-0.5 rounded px-0.5 text-[10px] leading-none opacity-0 transition-opacity group-hover:opacity-100 {isFavorite
        ? 'text-yellow-400'
        : 'text-zinc-500 hover:text-zinc-300'}"
    >
      {isFavorite ? '★' : '☆'}
    </button>
  {/if}
</div>
