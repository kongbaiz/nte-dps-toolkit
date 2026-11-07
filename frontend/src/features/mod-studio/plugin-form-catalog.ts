import { create } from "zustand";
import {
  invokeCommand,
  toCommandError,
  type CommandError,
} from "@/lib/tauri/command-error";
import type { PluginFormSnapshot } from "@/lib/tauri/generated/PluginFormSnapshot";
import { t } from "@/lib/i18n";

type Entry = {
  key: string;
  snapshot: PluginFormSnapshot | null;
  loading: boolean;
  error: CommandError | null;
};

export const usePluginFormCatalog = create<{
  entries: Record<string, Entry>;
  load: (
    file: string,
    identity: string,
    owner: string,
    reload?: boolean,
  ) => Promise<void>;
}>((set, get) => ({
  entries: {},
  load: async (file, identity, owner, reload = false) => {
    const key = JSON.stringify([identity, owner]);
    const previous = get().entries[file];
    if (
      previous?.key === key &&
      (previous.loading || (!reload && previous.snapshot))
    )
      return;
    set((state) => ({
      entries: {
        ...state.entries,
        [file]: {
          key,
          snapshot: previous?.snapshot ?? null,
          loading: true,
          error: null,
        },
      },
    }));
    try {
      const snapshot = await invokeCommand<PluginFormSnapshot>(
        "get_plugin_form",
        { file, expectedIdentity: identity },
      );
      if (get().entries[file]?.key === key)
        set((state) => ({
          entries: {
            ...state.entries,
            [file]: { key, snapshot, loading: false, error: null },
          },
        }));
    } catch (error) {
      if (get().entries[file]?.key === key)
        set((state) => ({
          entries: {
            ...state.entries,
            [file]: {
              key,
              snapshot: null,
              loading: false,
              error: toCommandError(error),
            },
          },
        }));
    }
  },
}));

export function pluginFormTitle(
  snapshot: PluginFormSnapshot | null | undefined,
  language: string,
) {
  return snapshot
    ? (snapshot.form.translations[language]?.[snapshot.form.title] ??
        t(snapshot.form.title))
    : t("Plugin controls");
}
