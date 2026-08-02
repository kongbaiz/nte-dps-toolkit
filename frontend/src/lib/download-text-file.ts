const REVOKE_DELAY_MS = 1_000;

export function downloadTextFile(
  filename: string,
  contents: string,
  mimeType = "application/json;charset=utf-8",
): void {
  const url = URL.createObjectURL(new Blob([contents], { type: mimeType }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();

  globalThis.setTimeout(() => URL.revokeObjectURL(url), REVOKE_DELAY_MS);
}
