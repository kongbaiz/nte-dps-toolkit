import { Channel, invoke } from "@tauri-apps/api/core";

import {
  StreamContractError,
  parseStreamDelivery,
  parseStreamSubscriptionReceipt,
  type StreamKind,
} from "@/lib/tauri/stream-contract";

export interface StreamTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface StreamOptions<Event> {
  transport: StreamTransport;
  streamKind: StreamKind;
  subscriptionId: string;
  subscribeCommand: string;
  unsubscribeCommand: string;
  subscribeArguments?: Record<string, unknown>;
  parseEvent(value: unknown): Event;
  onEvent(event: Event): void;
  onError(error: unknown): void;
}

export const tauriStreamTransport: StreamTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function subscribeStream<Event>(
  options: StreamOptions<Event>,
): () => Promise<void> {
  const {
    transport,
    streamKind,
    subscriptionId,
    subscribeCommand,
    unsubscribeCommand,
    subscribeArguments,
    parseEvent,
    onEvent,
    onError,
  } = options;

  let closed = false;
  let closePromise: Promise<void> | undefined;
  let rawReceipt: Promise<unknown> | undefined;
  let receiptReady = false;
  let pendingDelivery: unknown | undefined;
  let reportedFailure = false;

  const close = (): Promise<void> => {
    closed = true;
    closePromise ??= (async () => {
      if (rawReceipt === undefined) return;
      try {
        await rawReceipt;
      } catch {
        // Unsubscribe is idempotent and also covers a lost command response.
      }
      await transport.invoke(unsubscribeCommand, { subscriptionId });
    })();
    return reportedFailure ? closePromise.catch(() => undefined) : closePromise;
  };

  const failClosed = (error: unknown): void => {
    if (reportedFailure) return;
    reportedFailure = true;
    void close();
    try {
      onError(error);
    } catch {
      // Cleanup has already started; error reporting must not reopen the stream.
    }
  };

  const deliver = (value: unknown): void => {
    if (closed) return;
    try {
      for (const rawEvent of parseStreamDelivery(value)) {
        onEvent(parseEvent(rawEvent));
        if (closed) return;
      }
    } catch (error) {
      failClosed(error);
    }
  };

  const onMessage = (value: unknown): void => {
    if (closed) return;
    if (!receiptReady) {
      if (pendingDelivery !== undefined) {
        failClosed(new StreamContractError("stream started before subscription completed"));
        return;
      }
      pendingDelivery = value;
      return;
    }
    deliver(value);
  };

  const onEventChannel = transport.createChannel(onMessage);
  rawReceipt = transport.invoke(subscribeCommand, {
    ...subscribeArguments,
    subscriptionId,
    onEvent: onEventChannel,
  });
  void rawReceipt.then(
    (value) => {
      try {
        const receipt = parseStreamSubscriptionReceipt(value);
        if (
          receipt.subscriptionId !== subscriptionId ||
          receipt.streamKind !== streamKind
        ) {
          throw new StreamContractError("stream subscription identity mismatch");
        }
        receiptReady = true;
        if (pendingDelivery !== undefined) {
          const delivery = pendingDelivery;
          pendingDelivery = undefined;
          deliver(delivery);
        }
      } catch (error) {
        failClosed(error);
      }
    },
    failClosed,
  );

  return close;
}
