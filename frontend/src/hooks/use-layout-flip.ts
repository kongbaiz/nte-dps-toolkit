import { useLayoutEffect, useRef } from "react";

import { MOTION_DURATION, motionIsReduced } from "@/lib/motion";

interface LayoutPosition {
  left: number;
  top: number;
}

export function layoutDelta(previous: LayoutPosition, next: LayoutPosition) {
  return {
    x: previous.left - next.left,
    y: previous.top - next.top,
  };
}

export function useLayoutFlip<T extends HTMLElement>(layoutKey: string) {
  const elementRef = useRef<T>(null);
  const previousPosition = useRef<LayoutPosition | null>(null);
  const previousLayoutKey = useRef<string | null>(null);
  const activeAnimation = useRef<Animation | null>(null);

  useLayoutEffect(() => {
    const element = elementRef.current;
    if (element === null) return;

    const nextRect = element.getBoundingClientRect();
    const next = { left: nextRect.left, top: nextRect.top };
    const previous = previousPosition.current;
    const layoutChanged =
      previousLayoutKey.current !== null &&
      previousLayoutKey.current !== layoutKey;
    previousPosition.current = next;
    previousLayoutKey.current = layoutKey;
    if (!layoutChanged || previous === null || motionIsReduced()) return;

    const delta = layoutDelta(previous, next);
    if (Math.abs(delta.x) < 0.5 && Math.abs(delta.y) < 0.5) return;

    activeAnimation.current?.cancel();
    activeAnimation.current = element.animate(
      [
        { transform: `translate3d(${delta.x}px, ${delta.y}px, 0)` },
        { transform: "translate3d(0, 0, 0)" },
      ],
      {
        duration: MOTION_DURATION.slow + 40,
        easing: "cubic-bezier(0.16, 1, 0.3, 1)",
      },
    );
  });

  useLayoutEffect(
    () => () => {
      activeAnimation.current?.cancel();
    },
    [],
  );

  return elementRef;
}
