import { invoke } from '@tauri-apps/api/core';
import type { EmojiInfo, SyncStatus } from './emoji';

export async function trigger(emojiId: string): Promise<void> {
  await invoke<void>('trigger', { emojiId });
}

export async function listEmoji(): Promise<readonly EmojiInfo[]> {
  return invoke<EmojiInfo[]>('list_emoji');
}

export async function syncStatus(): Promise<SyncStatus> {
  return invoke<SyncStatus>('sync_status');
}
