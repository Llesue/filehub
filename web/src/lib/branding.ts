import { writable } from 'svelte/store';

export interface Branding {
  name: string;
  iconUrl: string | null;
  version: number;
  customCssVersion: number;
  loaded: boolean;
}

const initial: Branding = {
  name: 'FileHub',
  iconUrl: null,
  version: 0,
  customCssVersion: 0,
  loaded: false,
};

export const branding = writable<Branding>(initial);

export function brandMark(name: string): string {
  const trimmed = name.trim();
  return (trimmed.charAt(0) || 'F').toUpperCase();
}

export async function loadBranding(): Promise<void> {
  try {
    const response = await fetch('/api/v1/branding', {
      credentials: 'include',
      headers: { Accept: 'application/json' },
    });
    if (!response.ok) return;
    const data = (await response.json()) as {
      name?: string;
      version?: number;
      icon_url?: string | null;
      custom_css_version?: number;
    };
    const next: Branding = {
      name: data.name?.trim() || 'FileHub',
      iconUrl: data.icon_url ?? null,
      version: data.version ?? 0,
      customCssVersion: data.custom_css_version ?? 0,
      loaded: true,
    };
    branding.set(next);
    applyFavicon(next.iconUrl);
  } catch (error) {
    console.warn('failed to load branding', error);
  }
}

function applyFavicon(iconUrl: string | null): void {
  let link = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
  if (!link) {
    link = document.createElement('link');
    link.rel = 'icon';
    document.head.appendChild(link);
  }
  link.href = iconUrl ?? '/favicon.svg';
}
