export const MOTION_DURATION = {
  fast: 120,
  base: 180,
  slow: 260,
} as const;

export function motionIsReduced(): boolean {
  if (typeof document === "undefined") return true;
  if (document.documentElement.classList.contains("reduce-motion")) return true;
  return (
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false
  );
}

export function waitForMotion(
  duration: number = MOTION_DURATION.base,
): Promise<void> {
  if (motionIsReduced()) return Promise.resolve();
  return new Promise((resolve) => window.setTimeout(resolve, duration));
}

export function easeOutCubic(progress: number): number {
  const value = Math.min(1, Math.max(0, progress));
  return 1 - (1 - value) ** 3;
}

export function interpolateNumber(from: number, to: number, progress: number) {
  return from + (to - from) * easeOutCubic(progress);
}

export function startMotionViewTransition(update: () => void): void {
  if (motionIsReduced() || typeof document.startViewTransition !== "function") {
    update();
    return;
  }
  document.startViewTransition(update);
}
