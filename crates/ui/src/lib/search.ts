import type { EmojiInfo } from './emoji';

/**
 * Case-insensitive subset filter over `name` and `keywords`.
 * Empty / whitespace-only `query` returns the input unchanged.
 */
export function filterEmoji(items: readonly EmojiInfo[], query: string): readonly EmojiInfo[] {
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) {
    return items;
  }
  return items.filter((item) => {
    if (item.name.toLowerCase().includes(trimmed)) {
      return true;
    }
    return item.keywords.some((k) => k.toLowerCase().includes(trimmed));
  });
}

/**
 * Stable group ordering matching Unicode emoji groups.
 * Groups not in this list are appended in catalog order.
 */
const GROUP_ORDER: readonly string[] = [
  'Smileys & Emotion',
  'People & Body',
  'Animals & Nature',
  'Food & Drink',
  'Travel & Places',
  'Activities',
  'Objects',
  'Symbols',
  'Flags',
];

export interface GroupedEmoji {
  readonly group: string;
  readonly items: readonly EmojiInfo[];
}

/** Bucket items by `group`, preserving the canonical Unicode order. */
export function groupEmoji(items: readonly EmojiInfo[]): readonly GroupedEmoji[] {
  const buckets = new Map<string, EmojiInfo[]>();
  for (const item of items) {
    const bucket = buckets.get(item.group);
    if (bucket) {
      bucket.push(item);
    } else {
      buckets.set(item.group, [item]);
    }
  }
  const ordered: GroupedEmoji[] = [];
  for (const group of GROUP_ORDER) {
    const items = buckets.get(group);
    if (items) {
      ordered.push({ group, items });
      buckets.delete(group);
    }
  }
  for (const [group, items] of buckets) {
    ordered.push({ group, items });
  }
  return ordered;
}
