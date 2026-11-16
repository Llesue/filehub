const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function focusTrap(node: HTMLElement) {
  const previous = document.activeElement as HTMLElement | null;
  const timer = setTimeout(() => {
    const target =
      node.querySelector<HTMLElement>('[autofocus]') ??
      node.querySelector<HTMLElement>(FOCUSABLE);
    target?.focus();
  }, 0);

  const keydown = (event: KeyboardEvent) => {
    if (event.key !== 'Tab') return;
    const items = Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (item) => item.offsetParent !== null,
    );
    if (items.length === 0) {
      event.preventDefault();
      return;
    }
    const first = items.at(0);
    const last = items.at(-1);
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  node.addEventListener('keydown', keydown);
  return {
    destroy() {
      clearTimeout(timer);
      node.removeEventListener('keydown', keydown);
      previous?.focus?.();
    },
  };
}
