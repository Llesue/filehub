import { focusTrap } from './focusTrap';

export function overlayGuard(node: HTMLElement, close: () => void) {
  let pressed = false;
  const previous = document.activeElement as HTMLElement | null;
  const trap = focusTrap(node);
  const pointerdown = (event: PointerEvent) => {
    pressed = event.target === node;
  };
  const click = (event: MouseEvent) => {
    if (pressed && event.target === node) close();
    pressed = false;
  };
  const keydown = (event: KeyboardEvent) => {
    if (event.key === 'Escape') {
      event.stopPropagation();
      close();
    }
  };
  node.addEventListener('pointerdown', pointerdown);
  node.addEventListener('click', click);
  node.addEventListener('keydown', keydown);
  return {
    update(next: () => void) {
      close = next;
    },
    destroy() {
      node.removeEventListener('pointerdown', pointerdown);
      node.removeEventListener('click', click);
      node.removeEventListener('keydown', keydown);
      trap.destroy();
      previous?.focus?.();
    },
  };
}
