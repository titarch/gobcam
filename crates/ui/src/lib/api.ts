import { invoke } from '@tauri-apps/api/core';

export async function trigger(emojiId: string): Promise<void> {
  await invoke<void>('trigger', { emojiId });
}
