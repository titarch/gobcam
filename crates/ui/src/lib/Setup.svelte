<script lang="ts">
  import { runSetup, type SetupStatus } from './api';

  interface Props {
    status: SetupStatus;
    onComplete: () => void;
  }

  let { status, onComplete }: Props = $props();
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function handleSetup(): Promise<void> {
    busy = true;
    error = null;
    try {
      await runSetup();
      onComplete();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<main class="flex h-screen flex-col items-center justify-center gap-4 bg-zinc-900 p-6 text-zinc-100">
  <h1 class="text-lg font-semibold">Set up Gobcam</h1>
  <p class="text-center text-sm text-zinc-400">
    Gobcam needs to load the <code class="text-zinc-300">v4l2loopback</code> kernel module
    so video conferencing apps can pick it up as a camera. This is a one-time setup
    that requires admin privileges.
  </p>
  <p class="text-center text-xs text-zinc-500">
    Expected device: <code class="text-zinc-400">{status.output_path}</code>
  </p>

  {#if status.script_bundled}
    <button
      type="button"
      onclick={handleSetup}
      disabled={busy}
      class="rounded-lg bg-blue-600 px-5 py-2 text-sm font-medium text-white shadow transition hover:bg-blue-500 active:scale-95 disabled:opacity-60"
    >
      {busy ? 'Authenticating…' : 'Set up Gobcam'}
    </button>
    <p class="text-center text-xs text-zinc-500">
      You'll see a system password prompt.
    </p>
  {:else}
    <p class="text-center text-sm text-amber-400">
      The setup script isn't bundled with this build. Run
      <code class="text-zinc-300">just install-loopback</code> from the workspace,
      then restart the app.
    </p>
  {/if}

  {#if error}
    <div class="w-full rounded bg-red-900/60 p-2 text-xs" role="alert">
      {error}
    </div>
  {/if}
</main>
