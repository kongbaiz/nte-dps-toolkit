import { Maximize2, Minus, X } from "lucide-react";
import type { PointerEvent, ReactNode } from "react";

import { t } from "@/lib/i18n";
import { desktopWindowClient } from "@/lib/tauri/desktop-window-client";
import { cn } from "@/lib/utils";

export function DesktopTitlebar({
  title,
  status,
  onError,
}: {
  title: string;
  status?: ReactNode;
  onError?(error: unknown): void;
}) {
  const run = (operation: () => Promise<void>) => {
    void operation().catch((error: unknown) => {
      if (onError) onError(error);
      else console.error("desktop titlebar operation failed", error);
    });
  };
  const startDragging = (event: PointerEvent<HTMLElement>) => {
    if (
      event.button !== 0 ||
      !event.isPrimary ||
      (event.target as HTMLElement).closest("button") !== null
    )
      return;
    run(desktopWindowClient.startDragging);
  };

  return (
    <header className="desktop-titlebar" onPointerDown={startDragging}>
      <span className="absolute left-3 flex items-center">{status}</span>
      <strong className="pointer-events-none truncate px-32 text-base font-medium">
        {title}
      </strong>
      <div className="absolute right-1 flex h-full items-center">
        <TitleButton
          label={t("Minimize")}
          onClick={() => run(desktopWindowClient.minimize)}
        >
          <Minus />
        </TitleButton>
        <TitleButton
          label={t("Maximize")}
          onClick={() => run(desktopWindowClient.toggleMaximized)}
        >
          <Maximize2 />
        </TitleButton>
        <TitleButton
          label={t("Close")}
          danger
          onClick={() => run(desktopWindowClient.close)}
        >
          <X />
        </TitleButton>
      </div>
    </header>
  );
}

function TitleButton({
  label,
  danger = false,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      className={cn(
        "grid h-full w-10 place-items-center [&_svg]:size-3.5",
        danger ? "hover:bg-destructive hover:text-white" : "hover:bg-muted",
      )}
      aria-label={label}
      onClick={onClick}
    >
      {children}
    </button>
  );
}
