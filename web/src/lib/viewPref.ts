export type FileView = 'list' | 'thumb' | 'tile';

/**
 * Resolve a per-directory view preference. Directories without a saved value
 * fall back to the caller's default (tile in the browser).
 */
export function resolveViewPreference(value: string | null): FileView | null {
  if (value === 'list' || value === 'thumb' || value === 'tile') return value;
  if (value === 'grid') return 'thumb';
  return null;
}
