export type AsyncCleanup = () => void;

export function cleanupAsyncRegistration(
  registration: Promise<AsyncCleanup>,
  onError: (error: unknown) => void = (error) =>
    console.error("async registration failed", error),
): AsyncCleanup {
  let active = true;
  let cleanup: AsyncCleanup | null = null;
  void registration
    .then((registeredCleanup) => {
      if (active) cleanup = registeredCleanup;
      else registeredCleanup();
    })
    .catch(onError);
  return () => {
    active = false;
    cleanup?.();
    cleanup = null;
  };
}
