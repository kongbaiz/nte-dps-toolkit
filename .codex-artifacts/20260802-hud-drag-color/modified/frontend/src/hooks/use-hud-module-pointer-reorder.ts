import {
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";

import type { HudModuleId } from "@/lib/tauri/technical-contract";

export interface HudModulePointerTarget {
  module: HudModuleId;
  insertAfter: boolean;
}

export interface HudModulePointerBounds {
  left: number;
  right: number;
  top: number;
  bottom: number;
  height: number;
}

export function resolveHudModulePointerTarget(
  modules: readonly HudModuleId[],
  dragged: HudModuleId,
  clientX: number,
  clientY: number,
  boundsFor: (module: HudModuleId) => HudModulePointerBounds | null,
): HudModulePointerTarget | null {
  for (const module of modules) {
    if (module === dragged) continue;
    const bounds = boundsFor(module);
    if (
      bounds !== null &&
      clientX >= bounds.left &&
      clientX <= bounds.right &&
      clientY >= bounds.top &&
      clientY <= bounds.bottom
    ) {
      return {
        module,
        insertAfter: clientY >= bounds.top + bounds.height / 2,
      };
    }
  }
  return null;
}

export function useHudModulePointerReorder({
  modules,
  disabled,
  onMove,
}: {
  modules: readonly HudModuleId[];
  disabled: boolean;
  onMove: (
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ) => void | Promise<void>;
}) {
  const [draggedModule, setDraggedModule] = useState<HudModuleId | null>(null);
  const [dropTarget, setDropTarget] = useState<HudModulePointerTarget | null>(
    null,
  );
  const draggedModuleRef = useRef<HudModuleId | null>(null);
  const moduleRows = useRef(new Map<HudModuleId, HTMLElement>());
  const modulesRef = useRef(modules);
  const disabledRef = useRef(disabled);
  const onMoveRef = useRef(onMove);
  modulesRef.current = modules;
  disabledRef.current = disabled;
  onMoveRef.current = onMove;

  const resetPointerDrag = () => {
    draggedModuleRef.current = null;
    setDraggedModule(null);
    setDropTarget(null);
  };

  const pointerTarget = (clientX: number, clientY: number) => {
    const dragged = draggedModuleRef.current;
    if (dragged === null) return null;
    return resolveHudModulePointerTarget(
      modulesRef.current,
      dragged,
      clientX,
      clientY,
      (module) =>
        moduleRows.current.get(module)?.getBoundingClientRect() ?? null,
    );
  };

  const setModuleRow = (module: HudModuleId, row: HTMLElement | null) => {
    if (row === null) {
      moduleRows.current.delete(module);
    } else {
      moduleRows.current.set(module, row);
    }
  };

  const startPointerDrag = (
    event: ReactPointerEvent<HTMLElement>,
    module: HudModuleId,
  ) => {
    if (disabledRef.current || event.button !== 0 || !event.isPrimary) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    draggedModuleRef.current = module;
    setDraggedModule(module);
    setDropTarget(null);
  };

  const movePointerDrag = (event: ReactPointerEvent<HTMLElement>) => {
    if (draggedModuleRef.current === null) return;
    event.preventDefault();
    const target = pointerTarget(event.clientX, event.clientY);
    setDropTarget((current) =>
      current?.module === target?.module &&
      current?.insertAfter === target?.insertAfter
        ? current
        : target,
    );
  };

  const finishPointerDrag = (event: ReactPointerEvent<HTMLElement>) => {
    const dragged = draggedModuleRef.current;
    if (dragged === null) return;
    event.preventDefault();
    event.stopPropagation();
    const target = pointerTarget(event.clientX, event.clientY);
    resetPointerDrag();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (target !== null && !disabledRef.current) {
      void onMoveRef.current(dragged, target.module, target.insertAfter);
    }
  };

  const losePointerCapture = () => {
    if (draggedModuleRef.current !== null) resetPointerDrag();
  };

  return {
    draggedModule,
    dropTarget,
    setModuleRow,
    startPointerDrag,
    movePointerDrag,
    finishPointerDrag,
    cancelPointerDrag: resetPointerDrag,
    losePointerCapture,
  };
}
