<script lang="ts">
  import { trigger } from './api';

  interface Props {
    id: string;
    label: string;
    onError: (message: string) => void;
  }

  let { id, label, onError }: Props = $props();
  let busy = $state(false);

  async function handleClick(): Promise<void> {
    busy = true;
    try {
      await trigger(id);
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
  aria-label={id}
  class="aspect-square rounded-lg bg-zinc-800 text-4xl shadow transition hover:bg-zinc-700 active:scale-95 disabled:opacity-50"
>
  {label}
</button>
