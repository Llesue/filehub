import { describe, expect, it } from 'vitest';
import { resolveViewPreference } from './viewPref';

describe('resolveViewPreference', () => {
  it('returns the saved per-directory view', () => {
    expect(resolveViewPreference('list')).toBe('list');
    expect(resolveViewPreference('thumb')).toBe('thumb');
    expect(resolveViewPreference('tile')).toBe('tile');
  });

  it('maps the legacy grid value to thumbnails', () => {
    expect(resolveViewPreference('grid')).toBe('thumb');
  });

  it('ignores missing or unknown values', () => {
    expect(resolveViewPreference(null)).toBeNull();
    expect(resolveViewPreference('masonry')).toBeNull();
    expect(resolveViewPreference('columns')).toBeNull();
  });
});
