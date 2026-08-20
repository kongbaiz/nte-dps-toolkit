export function dismissLayerWhenClosed(
  open: boolean,
  onDismiss: () => void,
): void {
  if (!open) onDismiss();
}
