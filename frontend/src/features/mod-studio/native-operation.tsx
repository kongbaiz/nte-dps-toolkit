import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogBackdrop,
  AlertDialogPortal,
  AlertDialogPopup,
  AlertDialogTitle,
  AlertDialogDescription,
} from "@/components/ui/alert-dialog";
import type { ReleaseResult } from "@/lib/tauri/generated/ReleaseResult";
import { t } from "@/lib/i18n";

export function NativeResponse({ result }: { result: ReleaseResult }) {
  return (
    <details open className="rounded-lg border bg-muted/20 p-4">
      <summary className="cursor-pointer text-sm font-medium">
        {t("Native response")}{" "}
        <span className="font-mono text-xs text-muted-foreground">
          {result.totalBytes} B
          {result.truncated ? ` / ${t("Preview truncated")}` : ""}
        </span>
      </summary>
      <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg border bg-background p-3 text-xs">
        {result.preview}
      </pre>
      <p className="mt-2 text-xs text-muted-foreground">
        {t("Preview only. Export the full snapshot when needed.")}
      </p>
    </details>
  );
}

export function OperationConfirmation({
  message,
  pending,
  cancel,
  confirm,
}: {
  message: string | null;
  pending: boolean;
  cancel: () => void;
  confirm: () => void;
}) {
  return (
    <AlertDialog
      open={message !== null}
      onOpenChange={(open) => {
        if (!open && !pending) cancel();
      }}
    >
      <AlertDialogPortal>
        <AlertDialogBackdrop />
        <AlertDialogPopup className="left-1/2 top-1/2 flex w-[min(90vw,30rem)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-xl border bg-background p-6 shadow-lg">
          <AlertDialogTitle>{t("Confirm plugin operation")}</AlertDialogTitle>
          <AlertDialogDescription>{message}</AlertDialogDescription>
          <div className="flex justify-end gap-2">
            <Button variant="outline" disabled={pending} onClick={cancel}>
              {t("Cancel")}
            </Button>
            <Button disabled={pending} onClick={confirm}>
              {t("Confirm")}
            </Button>
          </div>
        </AlertDialogPopup>
      </AlertDialogPortal>
    </AlertDialog>
  );
}
