<script lang="ts">
  import EmojiButton from './lib/EmojiButton.svelte';
  import { EMOJI } from './lib/emoji';

  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  function showError(message: string): void {
    toast = message;
    if (toastTimer) {
      clearTimeout(toastTimer);
    }
    toastTimer = setTimeout(() => {
      toast = null;
    }, 3500);
  }
</script>

<main class="flex min-h-screen flex-col gap-3 p-3">
  <div class="grid grid-cols-2 gap-2">
    {#each EMOJI as emoji (emoji.id)}
      <EmojiButton id={emoji.id} label={emoji.label} onError={showError} />
    {/each}
  </div>
  {#if toast}
    <div class="rounded bg-red-900/60 p-2 text-sm" role="alert">{toast}</div>
  {/if}
</main>
