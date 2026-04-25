<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { trigger } from './api';
  import type { EmojiInfo } from './emoji';

  interface Props {
    item: EmojiInfo;
    onError: (message: string) => void;
  }

  let { item, onError }: Props = $props();
  let busy = $state(false);
  let imageOk = $state(true);

  let src = $derived(convertFileSrc(item.preview_path));

  async function handleClick(): Promise<void> {
    busy = true;
    try {
      await trigger(item.id);
    } catch (e: unknown) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<button
  type="button"
  onclick={handleClick}
  disabled={busy}
  title={item.name}
  aria-label={item.name}
  class="aspect-square flex items-center justify-center rounded-lg bg-zinc-800 shadow transition hover:bg-zinc-700 active:scale-95 disabled:opacity-50"
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
