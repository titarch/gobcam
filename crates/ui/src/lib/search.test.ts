import { describe, expect, it } from 'vitest';
import type { EmojiInfo } from './emoji';
import { filterEmoji, groupEmoji } from './search';

const fixture = (overrides: Partial<EmojiInfo>): EmojiInfo => ({
  id: 'fire',
  name: 'Fire',
  glyph: '🔥',
  group: 'Travel & Places',
  keywords: ['fire', 'flame', 'tool'],
  has_animated: true,
  preview_path: '/cache/fire.png',
  ...overrides,
});

describe('filterEmoji', () => {
  const items: readonly EmojiInfo[] = [
    fixture({ id: 'fire', name: 'Fire', keywords: ['fire', 'flame'] }),
    fixture({ id: 'thumbs_up', name: 'Thumbs up', keywords: ['hand', 'approve'] }),
    fixture({ id: 'red_heart', name: 'Red heart', keywords: ['love', 'red'] }),
  ];

  it('returns everything for empty query', () => {
    expect(filterEmoji(items, '')).toHaveLength(3);
    expect(filterEmoji(items, '   ')).toHaveLength(3);
  });

  it('matches by name (case-insensitive)', () => {
    const out = filterEmoji(items, 'FIR');
    expect(out.map((i) => i.id)).toEqual(['fire']);
  });

  it('matches by keyword', () => {
    const out = filterEmoji(items, 'love');
    expect(out.map((i) => i.id)).toEqual(['red_heart']);
  });

  it('returns nothing on no match', () => {
    expect(filterEmoji(items, 'xyzzy')).toEqual([]);
  });
});

describe('groupEmoji', () => {
  it('orders standard groups canonically', () => {
    const items = [
      fixture({ id: 'a', group: 'Symbols' }),
      fixture({ id: 'b', group: 'Smileys & Emotion' }),
      fixture({ id: 'c', group: 'Activities' }),
    ];
    const groups = groupEmoji(items);
    expect(groups.map((g) => g.group)).toEqual(['Smileys & Emotion', 'Activities', 'Symbols']);
  });

  it('appends unknown groups at the end in catalog order', () => {
    const items = [
      fixture({ id: 'a', group: 'Component' }),
      fixture({ id: 'b', group: 'Smileys & Emotion' }),
    ];
    const groups = groupEmoji(items);
    expect(groups.map((g) => g.group)).toEqual(['Smileys & Emotion', 'Component']);
  });
});
