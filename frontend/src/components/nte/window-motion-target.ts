export function isWindowMotionTarget(
  currentWindowLabel: string,
  targetWindowLabel: unknown,
): boolean {
  return targetWindowLabel === currentWindowLabel;
}
