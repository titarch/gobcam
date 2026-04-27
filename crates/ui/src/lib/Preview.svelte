<script lang="ts">
  import { previewUrl } from './api';

  interface Props {
    enabled: boolean;
  }
  let { enabled }: Props = $props();

  let url = $state<string | null>(null);

  // Re-fetch the daemon's MJPEG URL each time the preview enables.
  // The port can change across daemon respawns (settings changes), so
  // we don't cache it past a disable. Disabling unmounts the <img> so
  // the browser actually closes its TCP stream to the daemon.
  $effect(() => {
    if (!enabled) {
      url = null;
      return;
    }
    void previewUrl().then((u) => {
      url = u;
    });
  });
</script>

{#if enabled && url}
  <div class="border-b border-zinc-800 bg-black">
    <img
      src={url}
      alt="Preview"
      class="aspect-video w-full object-contain"
      decoding="async"
    />
  </div>
{/if}
