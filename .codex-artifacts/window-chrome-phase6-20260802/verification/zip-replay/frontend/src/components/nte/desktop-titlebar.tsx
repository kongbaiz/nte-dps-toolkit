import { Maximize2, Minus, Pin, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type { PointerEvent, ReactNode } from "react";

import { t } from "@/lib/i18n";
import { desktopWindowClient } from "@/lib/tauri/desktop-window-client";
import { cn } from "@/lib/utils";

export function DesktopTitlebar({
  title,
  status,
  alwaysOnTop,
  onAlwaysOnTopChange,
  onError,
}: {
  title: string;
  status?: ReactNode;
  alwaysOnTop?: boolean;
  onAlwaysOnTopChange?(enabled: boolean): Promise<void> | void;
  onError?(error: unknown): void;
}) {
  const controlledAlwaysOnTop = alwaysOnTop !== undefined;
  const [windowAlwaysOnTop, setWindowAlwaysOnTop] = useState(false);
  const [pinPending, setPinPending] = useState(false);
  const pinned = alwaysOnTop ?? windowAlwaysOnTop;
  const reportError = useCallback(
    (error: unknown) => {
      if (onError) onError(error);
      else console.error("desktop titlebar operation failed", error);
    },
    [onError],
  );
  const run = (operation: () => Promise<void>) => {
    void operation().catch(reportError);
  };

  useEffect(() => {
    if (controlledAlwaysOnTop) return;
    let active = true;
    void desktopWindowClient
      .isAlwaysOnTop()
      .then((enabled) => {
        if (active) setWindowAlwaysOnTop(enabled);
      })
      .catch(reportError);
    return () => {
      active = false;
    };
  }, [controlledAlwaysOnTop, reportError]);

  const toggleAlwaysOnTop = () => {
    if (pinPending) return;
    const enabled = !pinned;
    setPinPending(true);
    const operation = onAlwaysOnTopChange
      ? Promise.resolve(onAlwaysOnTopChange(enabled))
      : desktopWindowClient.setAlwaysOnTop(enabled);
    void operation
      .then(() => {
        if (!controlledAlwaysOnTop) setWindowAlwaysOnTop(enabled);
      })
      .catch(reportError)
      .finally(() => setPinPending(false));
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
      <strong className="pointer-events-none truncate px-40 text-base font-medium">
        {title}
      </strong>
      <div className="absolute right-1 flex h-full items-center">
        <TitleButton
          label={t("Always on top")}
          pressed={pinned}
          disabled={pinPending}
          onClick={toggleAlwaysOnTop}
        >
          <Pin className={cn(pinned && "fill-current")} />
        </TitleButton>
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
  pressed,
  disabled = false,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  pressed?: boolean;
  disabled?: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      className={cn(
        "grid h-full w-10 place-items-center transition-[color,background-color,opacity,transform] active:scale-90 disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-3.5",
        pressed && "bg-muted text-foreground",
        danger ? "hover:bg-destructive hover:text-white" : "hover:bg-muted",
      )}
      aria-label={label}
      aria-pressed={pressed}
      disabled={disabled}
      title={label}
      type="button"
      onClick={onClick}
    >
      {children}
    </button>
  );
}
