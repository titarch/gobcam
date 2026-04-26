import { invoke } from '@tauri-apps/api/core';
import type { EmojiInfo, SyncStatus } from './emoji';

export interface InputDevice {
  readonly device: string;
  readonly name: string;
}

export async function trigger(emojiId: string): Promise<void> {
  await invoke<void>('trigger', { emojiId });
}

export async function listEmoji(): Promise<readonly EmojiInfo[]> {
  return invoke<EmojiInfo[]>('list_emoji');
}

export async function syncStatus(): Promise<SyncStatus> {
  return invoke<SyncStatus>('sync_status');
}

export async function listInputs(): Promise<readonly InputDevice[]> {
  return invoke<InputDevice[]>('list_inputs');
}

export async function switchInput(device: string): Promise<void> {
  await invoke<void>('switch_input', { device });
}
