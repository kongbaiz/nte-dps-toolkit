import { Channel, invoke } from "@tauri-apps/api/core";

import { toCommandError, type CommandError } from "@/lib/tauri/command-error";
import type { StreamKind } from "@/lib/tauri/generated/StreamKind";
import type { SubscriptionReceipt } from "@/lib/tauri/generated/SubscriptionReceipt";

export interface StreamOptions<Event> {
  streamKind: StreamKind;
  subscribeCommand: string;
  unsubscribeCommand: string;
  subscribeArguments?: Record<string, unknown>;
  onEvent(event: Event): void;
  onError(error: CommandError): void;
}

/** Opens one ordered Tauri Channel and acknowledges each delivery after `onEvent`. */
export function subscribeStream<Event>(
  options: StreamOptions<Event>,
): () => Promise<void> {
  const {
    streamKind,
    subscribeCommand,
    unsubscribeCommand,
    subscribeArguments,
    onEvent,
    onError,
  } = options;

  const subscriptionId = crypto.randomUUID();
  let closed = false;
  let closePromise: Promise<void> | undefined;
  let streamGeneration: string | undefined;
  let deliverySequence = 0n;
  // Rust may deliver once before the receipt resolves; hold that event until then.
  let pendingEvent: { event: Event } | undefined;
  let reportedFailure = false;

  const failClosed = (error: unknown): void => {
    if (reportedFailure) return;
    reportedFailure = true;
    void close().catch(() => undefined);
    onError(toCommandError(error));
  };

  const deliver = (event: Event): void => {
    try {
      onEvent(event);
    } catch (error) {
      failClosed(error);
      return;
    }
    if (closed) return;
    deliverySequence += 1n;
    invoke("acknowledge_stream_delivery", {
      subscriptionId,
      streamKind,
      streamGeneration,
      deliverySequence: deliverySequence.toString(),
    }).catch((error: unknown) => {
      if (!closed) failClosed(error);
    });
  };

  const channel = new Channel<Event>();
  channel.onmessage = (event) => {
    if (closed) return;
    if (streamGeneration === undefined) pendingEvent = { event };
    else deliver(event);
  };

  const receipt = invoke<SubscriptionReceipt>(subscribeCommand, {
    ...subscribeArguments,
    subscriptionId,
    onEvent: channel,
  });
  receipt.then((value) => {
    streamGeneration = value.streamGeneration;
    if (pendingEvent === undefined || closed) return;
    const { event } = pendingEvent;
    pendingEvent = undefined;
    deliver(event);
  }, failClosed);

  const close = (): Promise<void> => {
    closed = true;
    closePromise ??= receipt
      .catch(() => undefined)
      // Unsubscribe is idempotent and also covers a lost subscribe response.
      .then(() => invoke<void>(unsubscribeCommand, { subscriptionId }));
    return closePromise;
  };

  return close;
}
