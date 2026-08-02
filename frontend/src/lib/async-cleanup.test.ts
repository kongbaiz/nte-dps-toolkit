import { describe, expect, it, vi } from "vitest";

import { cleanupAsyncRegistration } from "./async-cleanup";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

describe("cleanupAsyncRegistration", () => {
  it("cleans a registration that resolves after its effect was disposed", async () => {
    const registration = deferred<() => void>();
    const registeredCleanup = vi.fn();
    const cleanup = cleanupAsyncRegistration(registration.promise);

    cleanup();
    registration.resolve(registeredCleanup);
    await registration.promise;
    await Promise.resolve();

    expect(registeredCleanup).toHaveBeenCalledOnce();
  });

  it("cleans an active registration exactly once", async () => {
    const registeredCleanup = vi.fn();
    const cleanup = cleanupAsyncRegistration(
      Promise.resolve(registeredCleanup),
    );
    await Promise.resolve();

    cleanup();
    cleanup();

    expect(registeredCleanup).toHaveBeenCalledOnce();
  });
});
