export type FileView = 'list' | 'thumb' | 'tile';

/**
 * Resolve the view for a directory. A per-directory value wins; the global
 * value is only a default for directories that do not have one yet.
 */
export function resolveViewPreference(
  globalValue: string | null,
  pathValue: string | null,
): FileView | null {
  const value = pathValue ?? globalValue;
  if (value === 'list' || value === 'thumb' || value === 'tile') return value;
  if (value === 'grid') return 'thumb';
  return null;
}
