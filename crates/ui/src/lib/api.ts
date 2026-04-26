import { invoke } from '@tauri-apps/api/core';
import type { EmojiInfo, SyncStatus } from './emoji';

export interface Mode {
  readonly width: number;
  readonly height: number;
  readonly fps_num: number;
  readonly fps_den: number;
}

export interface InputDevice {
  readonly device: string;
  readonly name: string;
  readonly modes: readonly Mode[];
}

export interface AppliedSettings {
  readonly device: string;
  readonly width: number;
  readonly height: number;
  readonly fps_num: number;
  readonly fps_den: number;
  readonly preview: boolean;
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

export async function applySettings(settings: AppliedSettings): Promise<void> {
  await invoke<void>('apply_settings', { settings });
}

export async function previewPath(): Promise<string> {
  return invoke<string>('preview_path');
}

export interface CurrentSettings {
  readonly device: string;
  readonly width: number;
  readonly height: number;
  readonly fps_num: number;
  readonly fps_den: number;
  readonly preview: boolean;
}

export async function currentSettings(): Promise<CurrentSettings> {
  return invoke<CurrentSettings>('current_settings');
}

export interface SetupStatus {
  readonly required: boolean;
  readonly output_path: string;
  readonly script_bundled: boolean;
}

export async function setupStatus(): Promise<SetupStatus> {
  return invoke<SetupStatus>('setup_status');
}

export async function runSetup(): Promise<void> {
  await invoke<void>('run_setup');
}

export async function listRecents(): Promise<readonly string[]> {
  return invoke<string[]>('list_recents');
}

export interface HotkeySettings {
  readonly toggle: string | null;
  readonly repeat: string | null;
  readonly colorScheme: string;
}

interface CurrentHotkeysPayload {
  readonly recents: readonly string[];
  readonly favorites: readonly string[];
  readonly hotkey_toggle: string | null;
  readonly hotkey_repeat: string | null;
  readonly color_scheme: string;
}

export async function currentHotkeys(): Promise<HotkeySettings> {
  const payload = await invoke<CurrentHotkeysPayload>('current_hotkeys');
  return {
    toggle: payload.hotkey_toggle,
    repeat: payload.hotkey_repeat,
    colorScheme: payload.color_scheme,
  };
}

export async function setHotkeys(settings: HotkeySettings): Promise<void> {
  await invoke<void>('set_hotkeys', { toggle: settings.toggle, repeat: settings.repeat });
}

export async function listFavorites(): Promise<readonly string[]> {
  return invoke<string[]>('list_favorites');
}

export async function toggleFavorite(emojiId: string): Promise<boolean> {
  return invoke<boolean>('toggle_favorite', { emojiId });
}

export async function setColorScheme(scheme: string): Promise<void> {
  await invoke<void>('set_color_scheme', { scheme });
}

export async function quitApp(): Promise<void> {
  await invoke<void>('quit_app');
}

/** Format a Mode for the dropdown. Suppresses the `/1` denominator
 * for whole-fps modes so "30 fps" reads better than "30/1 fps". */
export function modeLabel(mode: Mode): string {
  const fps = mode.fps_den === 1 ? `${mode.fps_num}` : `${mode.fps_num}/${mode.fps_den}`;
  return `${mode.width}×${mode.height} @ ${fps} fps`;
}

/** Stable string key for a Mode, suitable for Svelte's `{#each}`
 * keying and HTML `<option value=…>`. */
export function modeKey(mode: Mode): string {
  return `${mode.width}x${mode.height}@${mode.fps_num}/${mode.fps_den}`;
}
