import catalog from '../../../../assets/fluent-catalog.json';
import type { EmojiInfo } from '../lib/emoji';

interface CatalogEntry {
  readonly id: string;
  readonly name: string;
  readonly glyph: string;
  readonly group: string;
  readonly keywords: readonly string[];
  readonly has_animated: boolean;
}

type InvokeArgs = Record<string, unknown> | undefined;

const catalogEntries = catalog as readonly CatalogEntry[];
const screenshotEmojiIds = [
  'fire',
  'red_heart',
  'party_popper',
  'sparkles',
  'thumbs_up',
  'clapping_hands',
  'face_with_tears_of_joy',
  'rocket',
  'star',
  'smiling_face_with_hearts',
  'folded_hands',
  'sunglasses',
  'saluting_face',
  'weary_face',
  'sun_with_face',
  'rainbow',
  'ghost',
  'bomb',
  'musical_note',
  'calendar',
  'compass',
  'pretzel',
  'sushi',
  'jack_o_lantern',
  'christmas_tree',
  'racing_car',
  'microphone',
  'musical_keyboard',
  'victory_hand',
  'selfie',
  'open_mailbox_with_lowered_flag',
  'clipboard',
  'loudly_crying_face',
  'rolling_on_the_floor_laughing',
  'eyes',
  'thinking_face',
  'ok_hand',
  'raising_hands',
  'hundred_points',
  'trophy',
  'camera_with_flash',
  'video_camera',
  'laptop',
  'keyboard',
  'light_bulb',
];

let recentIds = [
  'party_popper',
  'sparkles',
  'fire',
  'face_with_tears_of_joy',
  'thumbs_up',
  'clapping_hands',
];
let favoriteIds = ['fire', 'red_heart', 'party_popper', 'rocket', 'sparkles'];
let colorScheme = 'dark';

const items = screenshotEmojiIds.map(emojiInfo);

function emojiInfo(id: string): EmojiInfo {
  const entry = catalogEntries.find((candidate) => candidate.id === id);
  if (!entry) {
    throw new Error(`screenshot emoji missing from catalog: ${id}`);
  }
  return {
    id: entry.id,
    name: entry.name,
    glyph: entry.glyph,
    group: entry.group,
    keywords: entry.keywords,
    has_animated: entry.has_animated,
    preview_path: `/mock-cache/gobcam/previews/${entry.id}.png`,
    is_safe_mode_excluded: false,
  };
}

function stringArg(args: InvokeArgs, key: string): string {
  const value = args?.[key];
  return typeof value === 'string' ? value : '';
}

function rememberRecent(id: string): void {
  if (!id) {
    return;
  }
  recentIds = [id, ...recentIds.filter((candidate) => candidate !== id)].slice(0, 8);
}

function cloneList<T>(list: readonly T[]): T[] {
  return [...list];
}

export function convertFileSrc(assetPath: string): string {
  const normalized = assetPath.replaceAll('\\', '/');
  const fileName = normalized.split('/').filter(Boolean).at(-1);
  return fileName ? `/__gobcam-preview/${encodeURIComponent(fileName)}` : assetPath;
}

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  switch (command) {
    case 'setup_status':
      return {
        required: false,
        output_path: '/dev/video10',
        script_bundled: true,
      } as T;
    case 'list_emoji':
      return cloneList(items) as T;
    case 'sync_status':
      return {
        fetched: items.length,
        total: items.length,
        complete: true,
      } as T;
    case 'list_recents':
      return cloneList(recentIds) as T;
    case 'list_favorites':
      return cloneList(favoriteIds) as T;
    case 'current_hotkeys':
      return {
        recents: cloneList(recentIds),
        favorites: cloneList(favoriteIds),
        hotkey_toggle: 'Super+G',
        hotkey_repeat: 'Super+Space',
        color_scheme: colorScheme,
      } as T;
    case 'list_inputs':
      return [
        {
          device: '/dev/video0',
          name: 'Laptop Camera',
          modes: [
            { width: 1280, height: 720, fps_num: 30, fps_den: 1 },
            { width: 1920, height: 1080, fps_num: 30, fps_den: 1 },
          ],
        },
      ] as T;
    case 'current_settings':
      return {
        device: '/dev/video0',
        width: 1280,
        height: 720,
        fps_num: 30,
        fps_den: 1,
        preview: false,
        slot_count: 48,
        slot_dim: 256,
      } as T;
    case 'current_animations':
      return {
        lifetime_ms: 5000,
        fade_in_ms: 200,
        fade_out_start_ms: 3000,
        fade_out_ms: 2000,
        travel_px: 480,
        speed_jitter_pct: 0.25,
        start_x_fraction: 0.5,
        start_y_offset_px: 80,
        x_jitter_px: 220,
        direction_angle_deg: 90,
        apng_speed_multiplier: 1,
        max_concurrent: 48,
        drop_policy: 'drop_new',
        overrides: {},
      } as T;
    case 'preview_url':
      return null as T;
    case 'trigger':
      rememberRecent(stringArg(args, 'emojiId'));
      return undefined as T;
    case 'toggle_favorite': {
      const id = stringArg(args, 'emojiId');
      const isFavorite = !favoriteIds.includes(id);
      favoriteIds = isFavorite
        ? [id, ...favoriteIds]
        : favoriteIds.filter((candidate) => candidate !== id);
      return isFavorite as T;
    }
    case 'set_color_scheme':
      colorScheme = stringArg(args, 'scheme') || 'dark';
      return undefined as T;
    case 'apply_settings':
    case 'set_hotkeys':
    case 'set_animation_config':
    case 'run_setup':
    case 'quit_app':
      return undefined as T;
    default:
      throw new Error(`unhandled screenshot mock command: ${command}`);
  }
}
