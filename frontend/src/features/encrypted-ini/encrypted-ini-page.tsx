import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  ArrowDown,
  ArrowUp,
  BadgeCheck,
  FileKey2,
  FolderOpen,
  LockKeyhole,
  RefreshCw,
  Save,
  Search,
  Trash2,
  TriangleAlert,
  X,
} from "lucide-react";

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { t, tf } from "@/lib/i18n";
import type { EncryptedIniKey } from "@/lib/tauri/encrypted-ini-contract";
import { cn } from "@/lib/utils";

import {
  encryptedIniCenteredScrollOffset,
  encryptedIniLineColumn,
  findEncryptedIniMatches,
  nextEncryptedIniMatch,
  previousEncryptedIniMatch,
} from "./encrypted-ini-model";
import { type EncryptedIniNotice, useEncryptedIni } from "./use-encrypted-ini";

type PendingAction = "reload" | "clear";

const CONTROL_CLASS =
  "h-9 rounded-lg border bg-background px-3 text-sm outline-none transition-shadow focus:border-ring focus:ring-3 focus:ring-ring/25 disabled:cursor-not-allowed disabled:bg-muted disabled:text-muted-foreground";

export function EncryptedIniPage() {
  const model = useEncryptedIni();
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [plaintext, setPlaintext] = useState("");
  const [key, setKey] = useState<EncryptedIniKey>("global");
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
  const [currentMatch, setCurrentMatch] = useState<number | null>(null);
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(
    null,
  );

  useEffect(() => {
    if (!model.snapshot) return;
    setPlaintext(model.snapshot.plaintext);
    setKey(model.snapshot.key);
    setCurrentMatch(null);
  }, [model.snapshot]);

  const dirty =
    model.snapshot !== null &&
    (plaintext !== model.snapshot.plaintext || key !== model.snapshot.key);
  const matches = useMemo(
    () => findEncryptedIniMatches(plaintext, deferredSearch),
    [deferredSearch, plaintext],
  );
  const matchPosition =
    currentMatch === null ? null : (matches[currentMatch] ?? null);
  const lineColumn =
    matchPosition === null
      ? null
      : encryptedIniLineColumn(plaintext, matchPosition);

  useEffect(() => {
    setCurrentMatch(null);
  }, [deferredSearch, plaintext]);

  const jumpToMatch = (direction: "next" | "previous") => {
    const next =
      direction === "next"
        ? nextEncryptedIniMatch(currentMatch, matches.length)
        : previousEncryptedIniMatch(currentMatch, matches.length);
    setCurrentMatch(next);
    if (next === null) return;
    const start = matches[next];
    const end = start + deferredSearch.trim().length;
    requestAnimationFrame(() => {
      const editor = textareaRef.current;
      if (!editor) return;
      editor.focus({ preventScroll: true });
      editor.setSelectionRange(start, end);
      const position = encryptedIniLineColumn(plaintext, start);
      const style = window.getComputedStyle(editor);
      const lineHeight = Number.parseFloat(style.lineHeight) || 24;
      const fontSize = Number.parseFloat(style.fontSize) || 13;
      const characterWidth = fontSize * 0.62;
      const paddingTop = Number.parseFloat(style.paddingTop) || 0;
      const paddingLeft = Number.parseFloat(style.paddingLeft) || 0;
      editor.scrollTop = encryptedIniCenteredScrollOffset(
        paddingTop + (position.line - 1) * lineHeight,
        editor.clientHeight,
        editor.scrollHeight,
      );
      editor.scrollLeft = encryptedIniCenteredScrollOffset(
        paddingLeft + (position.column - 1) * characterWidth,
        editor.clientWidth,
        editor.scrollWidth,
      );
    });
  };

  const runAction = async (action: PendingAction) => {
    setPendingAction(null);
    if (action === "reload") {
      await model.reload();
    } else {
      await model.clear();
      setSearch("");
    }
  };

  const requestAction = (action: PendingAction) => {
    if (dirty) {
      setPendingAction(action);
    } else {
      void runAction(action);
    }
  };

  if (model.loading && model.snapshot === null) {
    return <EncryptedIniLoading />;
  }

  const snapshot = model.snapshot;
  const opened = snapshot?.opened ?? false;

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background">
      <header className="flex min-h-18 shrink-0 items-center justify-between gap-4 border-b px-6 py-3 max-[760px]:items-start max-[760px]:px-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h1 className="text-xl font-semibold">{t("Encrypted INI")}</h1>
            <Badge variant={opened ? "default" : "outline"}>
              {opened ? t("Open") : t("No file open")}
            </Badge>
          </div>
          <p className="mt-0.5 text-sm text-muted-foreground">
            {t(
              "Decrypt, inspect and re-encrypt local INI configuration files.",
            )}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2 max-[760px]:flex-wrap max-[760px]:justify-end">
          <Button disabled={model.busy} variant="outline" onClick={model.open}>
            <FolderOpen aria-hidden="true" />
            {t("Open INI")}
          </Button>
          <Button
            disabled={!opened || !dirty || model.busy}
            onClick={() => void model.save(plaintext, key)}
          >
            <Save aria-hidden="true" />
            {t("Save as Encrypted INI")}
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden px-6 py-4 max-[760px]:px-4">
        <div className="grid shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 border-b pb-3 max-[900px]:grid-cols-1">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-muted">
              <FileKey2
                aria-hidden="true"
                className="size-5 text-muted-foreground"
              />
            </div>
            <div className="min-w-0">
              <div
                className="truncate text-sm font-medium"
                title={snapshot?.displayPath ?? undefined}
              >
                {snapshot?.displayPath ?? t("No file open")}
              </div>
              <div className="mt-0.5 text-xs text-muted-foreground">
                {opened
                  ? tf("{} encrypted lines", [
                      String(snapshot?.encryptedLineCount ?? 0),
                    ])
                  : t("Open an encrypted INI to begin editing")}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2 max-[540px]:flex-wrap">
            <label className="flex items-center gap-2 text-sm text-muted-foreground">
              <span>{t("Save Key")}</span>
              <select
                className={cn(CONTROL_CLASS, "w-36")}
                value={key}
                onChange={(event) =>
                  setKey(event.target.value as EncryptedIniKey)
                }
              >
                <option value="global">{t("Global key")}</option>
                <option value="china">{t("China key")}</option>
              </select>
            </label>
            <Button
              aria-label={t("Reload")}
              disabled={!opened || model.busy}
              size="icon"
              variant="outline"
              onClick={() => requestAction("reload")}
            >
              <RefreshCw aria-hidden="true" />
            </Button>
            <Button
              aria-label={t("Clear")}
              disabled={(!opened && plaintext.length === 0) || model.busy}
              size="icon"
              variant="outline"
              onClick={() => requestAction("clear")}
            >
              <Trash2 aria-hidden="true" />
            </Button>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-2 max-[720px]:flex-wrap">
          <div className="relative min-w-48 flex-1 max-w-xl">
            <Search
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
            />
            <input
              aria-label={t("Search")}
              className={cn(CONTROL_CLASS, "w-full pl-9")}
              placeholder={t("Enter a config name or value")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </div>
          <Button
            disabled={matches.length === 0}
            size="sm"
            variant="outline"
            onClick={() => jumpToMatch("previous")}
          >
            <ArrowUp aria-hidden="true" />
            {t("Previous")}
          </Button>
          <Button
            disabled={matches.length === 0}
            size="sm"
            variant="outline"
            onClick={() => jumpToMatch("next")}
          >
            <ArrowDown aria-hidden="true" />
            {t("Next")}
          </Button>
          <div className="min-w-36 text-right font-mono text-xs text-muted-foreground max-[720px]:text-left">
            {search.trim().length === 0
              ? t("No search")
              : currentMatch !== null && lineColumn
                ? tf("{}/{}  line {} col {}", [
                    String(currentMatch + 1),
                    String(matches.length),
                    String(lineColumn.line),
                    String(lineColumn.column),
                  ])
                : tf("{} matches", [String(matches.length)])}
          </div>
        </div>

        <div className="relative min-h-0 flex-1 overflow-hidden rounded-xl border bg-muted/15">
          <textarea
            ref={textareaRef}
            aria-label={t("Decrypted INI plaintext")}
            className="size-full resize-none overflow-auto bg-transparent p-4 font-mono text-[13px] leading-6 outline-none select-text placeholder:text-muted-foreground/70 focus:ring-3 focus:ring-inset focus:ring-ring/20"
            placeholder={t(
              "After opening an encrypted INI, the decrypted plaintext appears here.",
            )}
            spellCheck={false}
            wrap="off"
            value={plaintext}
            onChange={(event) => setPlaintext(event.target.value)}
          />
          {!opened && plaintext.length === 0 ? (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
              <div className="flex flex-col items-center gap-2 text-center text-muted-foreground">
                <div className="flex size-12 items-center justify-center rounded-2xl bg-muted">
                  <LockKeyhole aria-hidden="true" className="size-6" />
                </div>
                <span className="text-sm">{t("No encrypted INI is open")}</span>
              </div>
            </div>
          ) : null}
        </div>

        <footer className="flex shrink-0 items-center justify-between gap-4 text-xs text-muted-foreground">
          <span>
            {dirty
              ? t("Unsaved changes")
              : opened
                ? t("Current content is saved or unchanged")
                : t("No file open")}
          </span>
          <span className="font-mono">
            {tf("{} characters", [String(plaintext.length)])}
          </span>
        </footer>
      </div>

      {model.notice ? (
        <EncryptedIniFloatingNotice
          notice={model.notice}
          onClose={model.clearNotice}
        />
      ) : null}
      {pendingAction ? (
        <EncryptedIniConfirmation
          action={pendingAction}
          onCancel={() => setPendingAction(null)}
          onConfirm={() => void runAction(pendingAction)}
        />
      ) : null}
    </section>
  );
}

function EncryptedIniLoading() {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 p-6">
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-16 w-full" />
      <Skeleton className="min-h-72 flex-1 w-full rounded-xl" />
    </div>
  );
}

function EncryptedIniFloatingNotice({
  notice,
  onClose,
}: {
  notice: EncryptedIniNotice;
  onClose: () => void;
}) {
  return createPortal(
    <div className="pointer-events-none fixed inset-x-0 top-4 z-[100] flex justify-center px-4">
      <Alert
        className="pointer-events-auto w-full max-w-lg bg-background pr-10 shadow-xl"
        variant={notice.kind === "error" ? "destructive" : "default"}
      >
        {notice.kind === "error" ? (
          <TriangleAlert aria-hidden="true" />
        ) : (
          <BadgeCheck aria-hidden="true" />
        )}
        <AlertTitle>{t(notice.titleKey)}</AlertTitle>
        <AlertDescription>
          {tf(notice.messageKey, notice.messageArguments)}
        </AlertDescription>
        <AlertAction>
          <Button
            aria-label={t("Dismiss")}
            size="icon-xs"
            variant="ghost"
            onClick={onClose}
          >
            <X aria-hidden="true" />
          </Button>
        </AlertAction>
      </Alert>
    </div>,
    document.body,
  );
}

function EncryptedIniConfirmation({
  action,
  onCancel,
  onConfirm,
}: {
  action: PendingAction;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return createPortal(
    <div className="fixed inset-0 z-[110] flex items-start justify-center bg-black/15 px-4 pt-4 backdrop-blur-[1px]">
      <Alert className="w-full max-w-lg bg-background shadow-xl">
        <TriangleAlert aria-hidden="true" />
        <AlertTitle>
          {t(action === "reload" ? "Confirm Reload" : "Confirm Clear")}
        </AlertTitle>
        <AlertDescription>
          {t("Unsaved changes will be discarded.")}
        </AlertDescription>
        <AlertAction className="flex gap-2">
          <Button size="sm" variant="outline" onClick={onCancel}>
            {t("Cancel")}
          </Button>
          <Button size="sm" onClick={onConfirm}>
            {t("Confirm")}
          </Button>
        </AlertAction>
      </Alert>
    </div>,
    document.body,
  );
}
