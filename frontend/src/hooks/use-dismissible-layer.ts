import { useEffect, useRef, type RefObject } from "react";

interface DismissibleLayerOptions {
  open: boolean;
  layerRef: RefObject<HTMLElement | null>;
  triggerRef?: RefObject<HTMLElement | null>;
  onDismiss: () => void;
}

interface ContainsNode {
  contains(target: Node): boolean;
}

export function dismissibleLayerEventIsOutside(
  target: Node,
  layer: ContainsNode | null,
  trigger: ContainsNode | null,
): boolean {
  return !layer?.contains(target) && !trigger?.contains(target);
}

export function useDismissibleLayer({
  open,
  layerRef,
  triggerRef,
  onDismiss,
}: DismissibleLayerOptions): void {
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;

  useEffect(() => {
    if (!open) return;

    const dismiss = () => dismissRef.current();
    const dismissOnPointerDown = (event: globalThis.PointerEvent) => {
      if (!(event.target instanceof Node)) return;
      if (
        dismissibleLayerEventIsOutside(
          event.target,
          layerRef.current,
          triggerRef?.current ?? null,
        )
      ) {
        dismiss();
      }
    };
    const dismissOnEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") dismiss();
    };

    document.addEventListener("pointerdown", dismissOnPointerDown, true);
    window.addEventListener("keydown", dismissOnEscape);
    window.addEventListener("blur", dismiss);
    window.addEventListener("scroll", dismiss, true);
    return () => {
      document.removeEventListener("pointerdown", dismissOnPointerDown, true);
      window.removeEventListener("keydown", dismissOnEscape);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("scroll", dismiss, true);
    };
  }, [layerRef, open, triggerRef]);
}
