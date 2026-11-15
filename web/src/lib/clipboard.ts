import { get } from 'svelte/store';
import { language, t } from './i18n';
import { showNotice } from './notices';
import { showToast } from './toast';

export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      showToast(t(get(language), '已复制到剪贴板'));
      return true;
    }
  } catch (error) {
    console.warn('clipboard API failed, falling back', error);
  }
  try {
    const area = document.createElement('textarea');
    area.value = text;
    area.style.position = 'fixed';
    area.style.opacity = '0';
    area.style.pointerEvents = 'none';
    document.body.appendChild(area);
    area.focus();
    area.select();
    const copied = document.execCommand('copy');
    area.remove();
    if (copied) showToast(t(get(language), '已复制到剪贴板'));
    else
      showNotice(
        t(get(language), '浏览器没有授予剪贴板权限，无法复制。'),
        'warning',
        t(get(language), '复制失败'),
      );
    return copied;
  } catch (error) {
    console.warn('could not copy text', error);
    showNotice(
      t(get(language), '浏览器没有授予剪贴板权限，无法复制。'),
      'warning',
      t(get(language), '复制失败'),
    );
    return false;
  }
}
