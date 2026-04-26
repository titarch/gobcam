<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { previewPath } from './api';

  interface Props {
    enabled: boolean;
  }
  let { enabled }: Props = $props();

  let path = $state<string | null>(null);
  let cacheBuster = $state(0);

  // Resolve the path lazily on first enable. It doesn't change once
  // resolved (depends only on $XDG_CACHE_HOME/$HOME).
  $effect(() => {
    if (enabled && path === null) {
      void previewPath().then((p) => {
        path = p;
      });
    }
  });

  // While enabled, bump `cacheBuster` every 200 ms so the `<img>`
  // re-fetches. The query-string change is enough — the file's bytes
  // are different on each daemon JPEG-encode.
  $effect(() => {
    if (!enabled) {
      return;
    }
    const id = setInterval(() => {
      cacheBuster = Date.now();
    }, 200);
    return () => {
      clearInterval(id);
    };
  });

  let src = $derived(path !== null ? `${convertFileSrc(path)}?t=${cacheBuster}` : null);
</script>

{#if enabled && src}
  <div class="border-b border-zinc-800 bg-black">
    <img
      src={src}
      alt="Preview"
      class="aspect-video w-full object-contain"
      decoding="async"
    />
  </div>
{/if}
