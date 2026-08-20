import {
  parseSkillsEvent,
  parseSkillsSnapshot,
  skillsError,
  type SkillsCommandError,
  type SkillsScope,
  type SkillsSnapshot,
} from "@/lib/tauri/skills-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";
import {
  subscribeStream,
  tauriStreamTransport,
} from "@/lib/tauri/stream-client";

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

const tauriTransport: SkillsTransport = tauriStreamTransport;

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
      return subscribeStream({
        transport,
        streamKind: "skills",
        subscriptionId,
        subscribeCommand: COMMANDS.subscribe,
        unsubscribeCommand: COMMANDS.unsubscribe,
        subscribeArguments: { scope },
        parseEvent: parseSkillsEvent,
        onEvent: onSnapshot,
        onError: (error) => onError(skillsError(error)),
      });
    },
  };
}

export const skillsClient = createSkillsClient();
