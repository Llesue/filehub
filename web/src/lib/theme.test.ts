import { describe, expect, it } from 'vitest';
import { resolveTheme } from './theme';

describe('resolveTheme', () => {
  it('uses an explicit stored light theme', () => {
    expect(resolveTheme('light', true)).toBe('light');
  });

  it('uses an explicit stored dark theme', () => {
    expect(resolveTheme('dark', false)).toBe('dark');
  });

  it('falls back to the system preference', () => {
    expect(resolveTheme(null, true)).toBe('dark');
    expect(resolveTheme(null, false)).toBe('light');
  });

  it('ignores an invalid stored value', () => {
    expect(resolveTheme('sepia', true)).toBe('dark');
  });
});
