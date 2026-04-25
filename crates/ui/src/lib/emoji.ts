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
}

/** Mirror of the daemon's `Response::SyncStatus`. */
export interface SyncStatus {
  readonly fetched: number;
  readonly total: number;
  readonly complete: boolean;
}
