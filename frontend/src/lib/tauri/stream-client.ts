import { Channel, invoke } from "@tauri-apps/api/core";

import {
  StreamContractError,
  parseStreamAckReceipt,
  parseStreamDelivery,
  parseStreamReadySignal,
  parseStreamSubscriptionReceipt,
  type StreamKind,
  type StreamReadySignal,
  type StreamSubscriptionReceipt,
} from "@/lib/tauri/stream-contract";

const READ_STREAM_DELIVERY_COMMAND = "read_stream_delivery";
const ACK_STREAM_DELIVERY_COMMAND = "ack_stream_delivery";

export interface AckedStreamTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface AckedStreamOptions<Event> {
  transport: AckedStreamTransport;
  streamKind: StreamKind;
  subscriptionId: string;
  subscribeCommand: string;
  unsubscribeCommand: string;
  subscribeArguments?: Record<string, unknown>;
  parseEvent(value: unknown): Event;
  onEvent(event: Event): void;
  onError(error: unknown): void;
}

export const tauriAckedStreamTransport: AckedStreamTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function subscribeAckedStream<Event>(
  options: AckedStreamOptions<Event>,
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
  let lifecycle = 0;
  let readInFlight = false;
  let inFlightSequence: bigint | undefined;
  let queuedSignal: StreamReadySignal | undefined;
  let streamGeneration: string | undefined;
  let lastAcknowledgedSequence: bigint | undefined;
  let receipt: StreamSubscriptionReceipt | undefined;
  let reportedFailure = false;
  let rawReceipt: Promise<unknown> | undefined;

  const close = (): Promise<void> => {
    if (!closed) {
      closed = true;
      lifecycle += 1;
    }
    closePromise ??= (async () => {
      const pendingReceipt = rawReceipt;
      if (pendingReceipt === undefined) return;
      try {
        await pendingReceipt;
      } catch {
        // The command may have activated the backend stream even if its
        // response was lost. The unsubscribe command is idempotent.
      }
      await transport.invoke(unsubscribeCommand, { subscriptionId });
    })();
    return reportedFailure ? closePromise.catch(() => undefined) : closePromise;
  };

  const failClosed = (error: unknown): void => {
    if (closed || reportedFailure) return;
    reportedFailure = true;
    void close().catch(() => undefined);
    try {
      onError(error);
    } catch {
      // Error reporting is an application boundary; cleanup has already begun.
    }
  };

  const identityArguments = (signal: StreamReadySignal) => ({
    streamKind,
    subscriptionId,
    streamGeneration: signal.streamGeneration,
    deliverySequence: signal.deliverySequence,
  });

  const processSignal = async (
    signal: StreamReadySignal,
    activeLifecycle: number,
  ): Promise<void> => {
    try {
      const rawDelivery = await transport.invoke(
        READ_STREAM_DELIVERY_COMMAND,
        identityArguments(signal),
      );
      if (closed || lifecycle !== activeLifecycle) return;

      const activeReceipt = await receiptPromise;
      if (
        closed ||
        lifecycle !== activeLifecycle ||
        activeReceipt === undefined
      ) {
        return;
      }
      const events = parseStreamDelivery(
        rawDelivery,
        activeReceipt.maxDeliveryBytes,
      );
      for (const value of events) {
        const event = parseEvent(value);
        if (closed || lifecycle !== activeLifecycle) return;
        onEvent(event);
      }
      if (closed || lifecycle !== activeLifecycle) return;

      const acknowledgement = parseStreamAckReceipt(
        await transport.invoke(
          ACK_STREAM_DELIVERY_COMMAND,
          identityArguments(signal),
        ),
      );
      if (!acknowledgement.accepted) {
        throw new StreamContractError("stream ACK was not accepted");
      }
      if (closed || lifecycle !== activeLifecycle) return;
      lastAcknowledgedSequence = BigInt(signal.deliverySequence);
    } catch (error) {
      failClosed(error);
    } finally {
      if (lifecycle === activeLifecycle) {
        readInFlight = false;
        inFlightSequence = undefined;
        const pending = queuedSignal;
        queuedSignal = undefined;
        if (!closed && pending !== undefined) {
          onSignal(pending);
        }
      }
    }
  };

  const onSignal = (value: unknown): void => {
    if (closed) return;
    let signal: StreamReadySignal;
    try {
      signal = parseStreamReadySignal(value);
    } catch (error) {
      failClosed(error);
      return;
    }
    if (
      signal.streamKind !== streamKind ||
      signal.subscriptionId !== subscriptionId
    ) {
      failClosed(new StreamContractError("stream ready identity mismatch"));
      return;
    }
    if (
      streamGeneration !== undefined &&
      signal.streamGeneration !== streamGeneration
    ) {
      return;
    }
    if (
      receipt !== undefined &&
      signal.streamGeneration !== receipt.streamGeneration
    ) {
      return;
    }
    const sequence = BigInt(signal.deliverySequence);
    if (readInFlight) {
      if (
        (inFlightSequence === undefined || sequence > inFlightSequence) &&
        (queuedSignal === undefined ||
          sequence > BigInt(queuedSignal.deliverySequence))
      ) {
        queuedSignal = signal;
      }
      return;
    }
    if (
      lastAcknowledgedSequence !== undefined &&
      sequence <= lastAcknowledgedSequence
    ) {
      return;
    }

    streamGeneration ??= signal.streamGeneration;
    readInFlight = true;
    inFlightSequence = sequence;
    const activeLifecycle = lifecycle;
    void processSignal(signal, activeLifecycle);
  };

  const onEventChannel = transport.createChannel(onSignal);
  if (closed) return close;
  rawReceipt = transport.invoke(subscribeCommand, {
    ...subscribeArguments,
    subscriptionId,
    onEvent: onEventChannel,
  });
  const receiptPromise: Promise<StreamSubscriptionReceipt | undefined> =
    rawReceipt.then(
      (value) => {
        try {
          const parsed = parseStreamSubscriptionReceipt(value);
          if (parsed.subscriptionId !== subscriptionId) {
            throw new StreamContractError(
              "stream subscription identity mismatch",
            );
          }
          if (parsed.streamKind !== streamKind) {
            throw new StreamContractError("stream subscription kind mismatch");
          }
          if (
            streamGeneration !== undefined &&
            parsed.streamGeneration !== streamGeneration
          ) {
            throw new StreamContractError(
              "stream subscription generation mismatch",
            );
          }
          receipt = parsed;
          streamGeneration ??= parsed.streamGeneration;
          return parsed;
        } catch (error) {
          failClosed(error);
          return undefined;
        }
      },
      (error: unknown) => {
        failClosed(error);
        return undefined;
      },
    );

  return close;
}
