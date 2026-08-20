"use client";

import { ContextMenu as ContextMenuPrimitive } from "@base-ui/react/context-menu";
import { Menu as MenuPrimitive } from "@base-ui/react/menu";

import { cn } from "@/lib/utils";

function Menu(props: MenuPrimitive.Root.Props) {
  return <MenuPrimitive.Root data-slot="menu" {...props} />;
}

function MenuTrigger(props: MenuPrimitive.Trigger.Props) {
  return <MenuPrimitive.Trigger data-slot="menu-trigger" {...props} />;
}

function MenuPortal(props: MenuPrimitive.Portal.Props) {
  return <MenuPrimitive.Portal data-slot="menu-portal" {...props} />;
}

function MenuPositioner({
  className,
  sideOffset = 4,
  ...props
}: MenuPrimitive.Positioner.Props) {
  return (
    <MenuPrimitive.Positioner
      className={cn("isolate z-50 outline-none", className)}
      data-slot="menu-positioner"
      sideOffset={sideOffset}
      {...props}
    />
  );
}

function MenuPopup({ className, ...props }: MenuPrimitive.Popup.Props) {
  return (
    <MenuPrimitive.Popup
      className={cn(
        "origin-(--transform-origin) rounded-lg border bg-popover p-1 text-sm text-popover-foreground shadow-xl outline-none data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95",
        className,
      )}
      data-slot="menu-popup"
      {...props}
    />
  );
}

function MenuItem({ className, ...props }: MenuPrimitive.Item.Props) {
  return (
    <MenuPrimitive.Item
      className={cn(
        "flex cursor-default items-center rounded-md px-2 py-1.5 text-left outline-none select-none data-highlighted:bg-muted data-disabled:pointer-events-none data-disabled:opacity-45",
        className,
      )}
      data-slot="menu-item"
      {...props}
    />
  );
}

function ContextMenu(props: ContextMenuPrimitive.Root.Props) {
  return <ContextMenuPrimitive.Root data-slot="context-menu" {...props} />;
}

function ContextMenuTrigger(props: ContextMenuPrimitive.Trigger.Props) {
  return (
    <ContextMenuPrimitive.Trigger data-slot="context-menu-trigger" {...props} />
  );
}

function ContextMenuPortal(props: ContextMenuPrimitive.Portal.Props) {
  return (
    <ContextMenuPrimitive.Portal data-slot="context-menu-portal" {...props} />
  );
}

function ContextMenuPositioner({
  className,
  ...props
}: ContextMenuPrimitive.Positioner.Props) {
  return (
    <ContextMenuPrimitive.Positioner
      className={cn("isolate z-50 outline-none", className)}
      data-slot="context-menu-positioner"
      {...props}
    />
  );
}

function ContextMenuPopup({
  className,
  ...props
}: ContextMenuPrimitive.Popup.Props) {
  return (
    <ContextMenuPrimitive.Popup
      className={cn(
        "origin-(--transform-origin) rounded-lg border bg-popover p-1 text-sm text-popover-foreground shadow-xl outline-none data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95",
        className,
      )}
      data-slot="context-menu-popup"
      {...props}
    />
  );
}

function ContextMenuItem({
  className,
  ...props
}: ContextMenuPrimitive.Item.Props) {
  return (
    <ContextMenuPrimitive.Item
      className={cn(
        "flex cursor-default items-center rounded-md px-2 py-1.5 text-left outline-none select-none data-highlighted:bg-muted data-disabled:pointer-events-none data-disabled:opacity-45",
        className,
      )}
      data-slot="context-menu-item"
      {...props}
    />
  );
}

export {
  ContextMenu,
  ContextMenuItem,
  ContextMenuPopup,
  ContextMenuPortal,
  ContextMenuPositioner,
  ContextMenuTrigger,
  Menu,
  MenuItem,
  MenuPopup,
  MenuPortal,
  MenuPositioner,
  MenuTrigger,
};
