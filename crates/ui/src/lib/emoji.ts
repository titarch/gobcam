/** Mirror of `gobcam_protocol::EmojiInfo`. */
export interface EmojiInfo {
  readonly id: string;
  readonly name: string;
  readonly glyph: string;
  readonly group: string;
  readonly keywords: readonly string[];
  readonly has_animated: boolean;
  /** Absolute path on disk where the daemon expects the static preview to be. */
  readonly preview_path: string;
  /** `true` when this id is on the safe-mode denylist; the picker
   * hides it whenever the user has Safe Mode toggled on. */
  readonly is_safe_mode_excluded: boolean;
}

/** Mirror of the daemon's `Response::SyncStatus`. */
export interface SyncStatus {
  readonly fetched: number;
  readonly total: number;
  readonly complete: boolean;
}
