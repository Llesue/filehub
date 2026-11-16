import { describe, expect, it } from 'vitest';
import { resolveViewPreference } from './viewPref';

describe('resolveViewPreference', () => {
  it('prefers the global view so directories share one layout', () => {
    expect(resolveViewPreference('thumb', 'list')).toBe('thumb');
  });

  it('falls back to the per-path value from older versions', () => {
    expect(resolveViewPreference(null, 'list')).toBe('list');
    expect(resolveViewPreference(null, 'grid')).toBe('thumb');
  });

  it('ignores unknown values', () => {
    expect(resolveViewPreference('masonry', 'columns')).toBeNull();
    expect(resolveViewPreference(null, null)).toBeNull();
  });
});
