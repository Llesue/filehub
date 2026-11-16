export type RelativeUnit =
  | 'just-now'
  | 'minute'
  | 'hour'
  | 'day'
  | 'week'
  | 'month'
  | 'year';

export interface RelativeParts {
  unit: RelativeUnit;
  count: number;
}

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;
const MONTH = 30 * DAY;
const YEAR = 365 * DAY;

/**
 * File lists show one consistent relative format regardless of age. Absolute
 * timestamps are only exposed through the tooltip.
 */
export function relativeTimeParts(seconds: number, nowMs = Date.now()): RelativeParts {
  const diff = Math.max(0, Math.floor(nowMs / 1000) - seconds);
  if (diff < MINUTE) return { unit: 'just-now', count: 0 };
  if (diff < HOUR) return { unit: 'minute', count: Math.floor(diff / MINUTE) };
  if (diff < DAY) return { unit: 'hour', count: Math.floor(diff / HOUR) };
  if (diff < WEEK) return { unit: 'day', count: Math.floor(diff / DAY) };
  if (diff < MONTH) return { unit: 'week', count: Math.floor(diff / WEEK) };
  if (diff < YEAR) return { unit: 'month', count: Math.floor(diff / MONTH) };
  return { unit: 'year', count: Math.floor(diff / YEAR) };
}

export function absoluteTime(seconds: number, locale: string): string {
  return new Date(seconds * 1000).toLocaleString(locale);
}
