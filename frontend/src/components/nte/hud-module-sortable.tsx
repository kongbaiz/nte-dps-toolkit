import {
  DndContext,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type Announcements,
  type DragEndEvent,
  type Modifier,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { startTransition, useOptimistic, type ReactNode } from "react";

import type { HudModule } from "@/lib/tauri/generated/HudModule";

const restrictToVerticalAxis: Modifier = ({ transform }) => ({
  ...transform,
  x: 0,
});

// Every sortable row also moves with the arrow keys, which is the accessible
// path; dnd-kit's built-in announcements are English-only, so stay silent.
const SILENT_ANNOUNCEMENTS: Announcements = {
  onDragStart: () => undefined,
  onDragOver: () => undefined,
  onDragEnd: () => undefined,
  onDragCancel: () => undefined,
};

/** Pointer-sortable HUD module list; shows the dropped order until Rust replies. */
export function HudModuleSortableList({
  modules,
  disabled,
  onMove,
  children,
}: {
  modules: readonly HudModule[];
  disabled: boolean;
  onMove: (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => Promise<void>;
  children: (modules: readonly HudModule[]) => ReactNode;
}) {
  const [displayed, showOrder] = useOptimistic(modules);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
  );

  const finishDrag = ({ active, over }: DragEndEvent) => {
    if (over === null || active.id === over.id) return;
    const dragged = active.id as HudModule;
    const target = over.id as HudModule;
    const from = displayed.indexOf(dragged);
    const to = displayed.indexOf(target);
    startTransition(async () => {
      showOrder(arrayMove([...displayed], from, to));
      await onMove(dragged, target, from < to);
    });
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      modifiers={[restrictToVerticalAxis]}
      accessibility={{ announcements: SILENT_ANNOUNCEMENTS }}
      onDragEnd={finishDrag}
    >
      <SortableContext
        items={[...displayed]}
        disabled={disabled}
        strategy={verticalListSortingStrategy}
      >
        {children(displayed)}
      </SortableContext>
    </DndContext>
  );
}
