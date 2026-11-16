import { describe, expect, it } from 'vitest';
import { detectBrowserLanguage } from './i18n';

describe('detectBrowserLanguage', () => {
  it('uses Chinese when the browser prefers Chinese', () => {
    expect(detectBrowserLanguage(['zh-CN', 'en-US'])).toBe('zh-CN');
    expect(detectBrowserLanguage(['zh-TW'])).toBe('zh-CN');
  });

  it('uses English for non-Chinese browsers', () => {
    expect(detectBrowserLanguage(['en-US', 'zh-CN'])).toBe('en');
    expect(detectBrowserLanguage(['de-DE'])).toBe('en');
  });

  it('falls back to Chinese when no language is available', () => {
    expect(detectBrowserLanguage(undefined)).toBe('zh-CN');
    expect(detectBrowserLanguage([])).toBe('zh-CN');
  });
});
