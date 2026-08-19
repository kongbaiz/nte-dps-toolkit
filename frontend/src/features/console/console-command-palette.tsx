import { Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import {
  Dialog,
  DialogBackdrop,
  DialogClose,
  DialogPopup,
  DialogPortal,
  DialogTitle,
} from "@/components/ui/dialog";

import { t, useTranslationRevision } from "@/lib/i18n";
import { cn } from "@/lib/utils";

import {
  CONSOLE_COMMANDS,
  consoleCommandRowClasses,
  filterConsoleCommands,
  nextEnabledCommandIndex,
  type ConsoleCommandAction,
} from "./console-command-palette-model";

export function ConsoleCommandPalette({
  open,
  onOpenChange,
  onExecute,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  onExecute(action: ConsoleCommandAction): Promise<void> | void;
}) {
  const translationRevision = useTranslationRevision();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const filtered = useMemo(() => {
    void translationRevision;
    return filterConsoleCommands(CONSOLE_COMMANDS, query, t);
  }, [query, translationRevision]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelected(nextEnabledCommandIndex(CONSOLE_COMMANDS, -1, 1));
  }, [open]);

  useEffect(() => {
    setSelected((current) => {
      if (
        filtered[current] !== undefined &&
        filtered[current].action.kind !== "unavailable"
      ) {
        return current;
      }
      return nextEnabledCommandIndex(filtered, -1, 1);
    });
  }, [filtered]);

  const execute = (index: number) => {
    const command = filtered[index];
    if (command === undefined || command.action.kind === "unavailable") return;
    onOpenChange(false);
    void onExecute(command.action);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogPortal>
        <DialogBackdrop className="bg-black/45" />
        <DialogPopup
          className="ui-motion-dialog top-[clamp(1rem,12vh,4.5rem)] left-1/2 flex max-h-[min(32rem,calc(100vh-2rem))] w-[calc(100vw-1.5rem)] max-w-xl -translate-x-1/2 flex-col overflow-hidden rounded-xl border bg-popover text-popover-foreground shadow-2xl"
          initialFocus={inputRef}
        >
          <DialogTitle className="sr-only">{t("Command palette")}</DialogTitle>
          <div className="flex items-center gap-2 border-b px-3 py-2">
            <Search
              className="size-4 text-muted-foreground"
              aria-hidden="true"
            />
            <input
              ref={inputRef}
              aria-label={t("Search commands")}
              className="h-9 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground select-text"
              placeholder={t("Search commands")}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                  event.preventDefault();
                  setSelected((current) =>
                    nextEnabledCommandIndex(
                      filtered,
                      current,
                      event.key === "ArrowDown" ? 1 : -1,
                    ),
                  );
                } else if (event.key === "Enter") {
                  event.preventDefault();
                  execute(selected);
                }
              }}
            />
            <DialogClose
              aria-label={t("Close")}
              className="grid size-8 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
              type="button"
            >
              <X className="size-4" aria-hidden="true" />
            </DialogClose>
          </div>
          <div className="min-h-0 overflow-y-auto p-2" role="listbox">
            {filtered.length === 0 ? (
              <p className="px-3 py-8 text-center text-sm text-muted-foreground">
                {t("No matching commands")}
              </p>
            ) : (
              <div className="flex flex-col gap-1">
                {filtered.map((command, index) => {
                  const disabled = command.action.kind === "unavailable";
                  const selectedRow = index === selected && !disabled;
                  const rowClasses = consoleCommandRowClasses(
                    disabled,
                    selectedRow,
                  );
                  return (
                    <button
                      aria-disabled={disabled}
                      aria-selected={index === selected}
                      className={cn(
                        "flex min-h-11 w-full items-center justify-between gap-3 rounded-md px-3 py-2 text-left transition-[color,background-color,transform] duration-150 ease-out active:scale-[0.99]",
                        rowClasses.row,
                      )}
                      key={command.id}
                      onClick={() => execute(index)}
                      onMouseEnter={() => {
                        if (!disabled) setSelected(index);
                      }}
                      role="option"
                      type="button"
                    >
                      <span className="min-w-0">
                        <span className="block truncate text-sm">
                          {t(command.titleKey)}
                        </span>
                        <span
                          className={cn(
                            "block truncate text-[11px]",
                            rowClasses.secondary,
                          )}
                        >
                          {t(command.categoryKey)}
                        </span>
                      </span>
                      <span
                        className={cn(
                          "shrink-0 text-[11px]",
                          rowClasses.secondary,
                        )}
                      >
                        {disabled ? t("Disabled") : ""}
                      </span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </DialogPopup>
      </DialogPortal>
    </Dialog>
  );
}
