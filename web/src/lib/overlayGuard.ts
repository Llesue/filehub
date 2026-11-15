export function overlayGuard(node: HTMLElement, close: () => void) {
  let pressed = false;
  const pointerdown = (event: PointerEvent) => {
    pressed = event.target === node;
  };
  const click = (event: MouseEvent) => {
    if (pressed && event.target === node) close();
    pressed = false;
  };
  node.addEventListener('pointerdown', pointerdown);
  node.addEventListener('click', click);
  return {
    update(next: () => void) {
      close = next;
    },
    destroy() {
      node.removeEventListener('pointerdown', pointerdown);
      node.removeEventListener('click', click);
    },
  };
}
