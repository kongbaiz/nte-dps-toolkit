import { Check, LoaderCircle, TriangleAlert, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { t } from "@/lib/i18n";
import { cn } from "@/lib/utils";

interface ActionNoticeProps {
  status: "pending" | "success" | "error";
  message: string;
  detail?: string;
  actionLabel?: string;
  onAction?: () => void;
  onDismiss?: () => void;
  compact?: boolean;
}

export function ActionNotice({
  status,
  message,
  detail,
  actionLabel,
  onAction,
  onDismiss,
  compact = false,
}: ActionNoticeProps) {
  const Icon =
    status === "pending"
      ? LoaderCircle
      : status === "success"
        ? Check
        : TriangleAlert;
  return (
    <section
      className={cn(
        "motion-action-notice flex items-center gap-2 rounded-xl border bg-background/95 shadow-lg backdrop-blur",
        compact ? "px-2.5 py-1.5 text-xs" : "px-4 py-2.5 text-sm",
        status === "success" && "border-emerald-500/35",
        status === "error" && "border-destructive/35",
      )}
      data-status={status}
      data-compact={compact || undefined}
      role={status === "error" ? "alert" : "status"}
    >
      <span
        className={cn(
          "motion-notice-icon grid shrink-0 place-items-center",
          status === "pending" && "animate-spin",
        )}
      >
        <Icon
          className={cn(
            compact ? "size-3.5" : "size-4",
            status === "success" && "text-emerald-600",
            status === "error" && "text-destructive",
          )}
          aria-hidden="true"
        />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{message}</span>
        {detail === undefined ? null : (
          <span className="block truncate text-muted-foreground">{detail}</span>
        )}
      </span>
      {actionLabel !== undefined && onAction !== undefined ? (
        <Button size="sm" variant="outline" onClick={onAction}>
          {actionLabel}
        </Button>
      ) : null}
      {onDismiss === undefined ? null : (
        <button
          type="button"
          className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          aria-label={t("Close")}
          onClick={onDismiss}
        >
          <X className="size-3.5" aria-hidden="true" />
        </button>
      )}
    </section>
  );
}
