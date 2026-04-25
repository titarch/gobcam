export interface Emoji {
  readonly id: string;
  readonly label: string;
}

export const EMOJI: readonly Emoji[] = [
  { id: 'thumbs_up', label: '👍' },
  { id: 'red_heart', label: '❤️' },
  { id: 'fire', label: '🔥' },
  { id: 'party_popper', label: '🎉' },
  { id: 'smiling_face_with_smiling_eyes', label: '😊' },
];
