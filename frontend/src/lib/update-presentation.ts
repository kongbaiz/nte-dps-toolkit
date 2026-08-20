import type { UpdateComponentId } from "@/lib/tauri/settings-contract";

const KIBIBYTE = 1_024n;
const MEBIBYTE = KIBIBYTE * KIBIBYTE;
const GIBIBYTE = MEBIBYTE * KIBIBYTE;

const BYTE_UNITS = [
  { size: GIBIBYTE, suffix: "GiB" },
  { size: MEBIBYTE, suffix: "MiB" },
  { size: KIBIBYTE, suffix: "KiB" },
] as const;

export function updateComponentLabelKey(
  component: UpdateComponentId,
): "Application" | "Mod loader" {
  return component === "app" ? "Application" : "Mod loader";
}

export function updateProgressPercent(
  downloaded: string,
  total: string,
): number | null {
  const downloadedBytes = parseByteCount(downloaded);
  const totalBytes = parseByteCount(total);
  if (downloadedBytes === null || totalBytes === null || totalBytes === 0n) {
    return null;
  }

  const boundedDownloaded =
    downloadedBytes > totalBytes ? totalBytes : downloadedBytes;
  return Number((boundedDownloaded * 100n) / totalBytes);
}

export function formatByteCount(value: string): string {
  const bytes = parseByteCount(value);
  if (bytes === null) return value;

  const unit = BYTE_UNITS.find((candidate) => bytes >= candidate.size);
  if (unit === undefined) return `${bytes} B`;

  const tenths = (bytes * 10n) / unit.size;
  return `${tenths / 10n}.${tenths % 10n} ${unit.suffix}`;
}

export function formatUpdateByteProgress(
  downloaded: string,
  total: string,
): string {
  const downloadedLabel = formatByteCount(downloaded);
  const totalBytes = parseByteCount(total);
  if (totalBytes === null || totalBytes === 0n) return downloadedLabel;
  return `${downloadedLabel} / ${formatByteCount(total)}`;
}

function parseByteCount(value: string): bigint | null {
  if (!/^\d+$/.test(value)) return null;
  return BigInt(value);
}
