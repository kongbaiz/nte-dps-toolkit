import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useReducedMotionConfig } from "motion/react";
import type { CSSProperties } from "react";

import { MOTION_DURATION } from "@/lib/motion";
import type { HudModule } from "@/lib/tauri/generated/HudModule";

/** One row inside `HudModuleSortableList`. */
export function useHudModuleSortable(module: HudModule) {
  const reduceMotion = useReducedMotionConfig();
  const {
    setNodeRef,
    setActivatorNodeRef,
    listeners,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: module,
    transition: reduceMotion
      ? null
      : { duration: MOTION_DURATION.slow, easing: "ease-out" },
  });
  const rowStyle: CSSProperties = {
    transform: CSS.Translate.toString(transform),
    transition,
  };
  return {
    rowRef: setNodeRef,
    rowStyle,
    handleRef: setActivatorNodeRef,
    handleListeners: listeners,
    dragging: isDragging,
  };
}
