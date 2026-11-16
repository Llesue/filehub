export type FileView = 'list' | 'thumb' | 'tile';

/**
 * Resolve the view for a directory. The global preference wins so switching
 * directories keeps the user's chosen layout; old per-path values still work
 * as a fallback for users upgrading from 0.1.3.
 */
export function resolveViewPreference(
  globalValue: string | null,
  pathValue: string | null,
): FileView | null {
  const value = globalValue ?? pathValue;
  if (value === 'list' || value === 'thumb' || value === 'tile') return value;
  if (value === 'grid') return 'thumb';
  return null;
}
