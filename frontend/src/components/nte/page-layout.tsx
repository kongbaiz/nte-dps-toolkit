import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

export function PageHeader({ className, ...props }: ComponentProps<"header">) {
  return (
    <header
      data-slot="page-header"
      className={cn("ui-page-header", className)}
      {...props}
    />
  );
}

export function PageTitle({ className, ...props }: ComponentProps<"h1">) {
  return (
    <h1
      data-slot="page-title"
      className={cn("ui-page-title", className)}
      {...props}
    />
  );
}

export function PageDescription({ className, ...props }: ComponentProps<"p">) {
  return (
    <p
      data-slot="page-description"
      className={cn("ui-page-description", className)}
      {...props}
    />
  );
}

export function PageScrollArea({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="page-scroll"
      className={cn("ui-page-scroll", className)}
      {...props}
    />
  );
}

export function PreferencesGrid({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      data-slot="preferences-grid"
      className={cn("ui-preferences-grid", className)}
      {...props}
    />
  );
}
