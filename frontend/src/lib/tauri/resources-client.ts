import { invoke } from "@tauri-apps/api/core";

import {
  parseResourcesSnapshot,
  resourcesError,
  type ResourcesSnapshot,
} from "@/lib/tauri/resources-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";

const COMMAND = "get_resources_snapshot";

interface ResourcesTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
}

export interface ResourcesClient {
  getSnapshot(): Promise<ResourcesSnapshot>;
}

const tauriTransport: ResourcesTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
};

export function createResourcesClient(
  transport: ResourcesTransport = tauriTransport,
): ResourcesClient {
  return {
    getSnapshot: async () => {
      try {
        return parseResourcesSnapshot(await transport.invoke(COMMAND));
      } catch (error) {
        if (error instanceof TechnicalContractError) throw error;
        throw resourcesError(error);
      }
    },
  };
}

export const resourcesClient = createResourcesClient();
