export function suppressBrowserContextMenu(event: Event): void {
  event.preventDefault();
}

export function installBrowserContextMenuSuppression(
  target: EventTarget = window,
): () => void {
  target.addEventListener("contextmenu", suppressBrowserContextMenu, {
    capture: true,
  });
  return () =>
    target.removeEventListener("contextmenu", suppressBrowserContextMenu, {
      capture: true,
    });
}
