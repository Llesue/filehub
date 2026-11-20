import { focusTrap } from './focusTrap';

let scrollLocks = 0;
let previousOverflow = '';

function lockScroll(): void {
  if (scrollLocks === 0) {
    previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
  }
  scrollLocks += 1;
}

function unlockScroll(): void {
  scrollLocks = Math.max(0, scrollLocks - 1);
  if (scrollLocks === 0) {
    document.body.style.overflow = previousOverflow;
  }
}

export function overlayGuard(node: HTMLElement, close: () => void) {
  let pressed = false;
  const previous = document.activeElement as HTMLElement | null;
  const trap = focusTrap(node);
  lockScroll();
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
      unlockScroll();
      previous?.focus?.();
    },
  };
}
