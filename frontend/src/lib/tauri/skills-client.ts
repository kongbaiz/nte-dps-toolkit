import { Channel, invoke } from "@tauri-apps/api/core";

import {
  parseSkillsEvent,
  parseSkillsSnapshot,
  skillsError,
  type SkillsCommandError,
  type SkillsScope,
  type SkillsSnapshot,
} from "@/lib/tauri/skills-contract";
import {
  parseSubscriptionReceipt,
  TechnicalContractError,
} from "@/lib/tauri/technical-contract";

const COMMANDS = {
  getSnapshot: "get_skills_snapshot",
  subscribe: "subscribe_skills",
  unsubscribe: "unsubscribe_skills",
} as const;

interface SkillsTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface SkillsClient {
  getSnapshot(scope: SkillsScope): Promise<SkillsSnapshot>;
  subscribe(
    scope: SkillsScope,
    onSnapshot: (snapshot: SkillsSnapshot) => void,
    onError: (error: SkillsCommandError) => void,
  ): () => Promise<void>;
}

const tauriTransport: SkillsTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function createSkillsClient(
  transport: SkillsTransport = tauriTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
): SkillsClient {
  return {
    getSnapshot: async (scope) => {
      try {
        return parseSkillsSnapshot(
          await transport.invoke(COMMANDS.getSnapshot, { scope }),
        );
      } catch (error) {
        if (error instanceof TechnicalContractError) throw error;
        throw skillsError(error);
      }
    },
    subscribe: (scope, onSnapshot, onError) => {
      const subscriptionId = createSubscriptionId();
      let closed = false;
      const onEvent = transport.createChannel((message) => {
        if (closed) return;
        try {
          onSnapshot(parseSkillsEvent(message));
        } catch (error) {
          onError(skillsError(error));
        }
      });
      const receipt = transport
        .invoke(COMMANDS.subscribe, { subscriptionId, scope, onEvent })
        .then(parseSubscriptionReceipt)
        .catch((error: unknown) => {
          if (!closed) onError(skillsError(error));
          return undefined;
        });
      return async () => {
        if (closed) return;
        closed = true;
        const activeReceipt = await receipt;
        if (activeReceipt) {
          await transport.invoke(COMMANDS.unsubscribe, {
            subscriptionId: activeReceipt.subscriptionId,
          });
        }
      };
    },
  };
}

export const skillsClient = createSkillsClient();
