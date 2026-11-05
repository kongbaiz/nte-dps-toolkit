import { useEffect, useEffectEvent } from "react";

/** Closes a transient popup when the window loses focus while it is open. */
export function useCloseOnWindowBlur(open: boolean, close: () => void) {
  const onBlur = useEffectEvent(close);
  useEffect(() => {
    if (!open) return;
    const listener = () => onBlur();
    window.addEventListener("blur", listener);
    return () => window.removeEventListener("blur", listener);
  }, [open]);
}
