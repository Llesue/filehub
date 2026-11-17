import { describe, expect, it } from 'vitest';
import { resolveViewPreference } from './viewPref';

describe('resolveViewPreference', () => {
  it('prefers the per-directory view', () => {
    expect(resolveViewPreference('thumb', 'list')).toBe('list');
  });

  it('falls back to the global default', () => {
    expect(resolveViewPreference('thumb', null)).toBe('thumb');
    expect(resolveViewPreference(null, 'grid')).toBe('thumb');
  });

  it('ignores unknown values', () => {
    expect(resolveViewPreference('masonry', 'columns')).toBeNull();
    expect(resolveViewPreference(null, null)).toBeNull();
  });
});
