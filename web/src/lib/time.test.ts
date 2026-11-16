import { describe, expect, it } from 'vitest';
import { relativeTimeParts } from './time';

const NOW = Date.UTC(2026, 10, 16, 12, 0, 0);

describe('relativeTimeParts', () => {
  const at = (secondsAgo: number) =>
    relativeTimeParts(NOW / 1000 - secondsAgo, NOW);

  it('uses every relative bucket', () => {
    expect(at(10)).toEqual({ unit: 'just-now', count: 0 });
    expect(at(120)).toEqual({ unit: 'minute', count: 2 });
    expect(at(3 * 3600)).toEqual({ unit: 'hour', count: 3 });
    expect(at(3 * 86400)).toEqual({ unit: 'day', count: 3 });
    expect(at(10 * 86400)).toEqual({ unit: 'week', count: 1 });
    expect(at(45 * 86400)).toEqual({ unit: 'month', count: 1 });
    expect(at(800 * 86400)).toEqual({ unit: 'year', count: 2 });
  });

  it('never falls back to an absolute date', () => {
    const old = at(5 * 365 * 86400);
    expect(old.unit).toBe('year');
    expect(old.count).toBeGreaterThanOrEqual(5);
  });
});
