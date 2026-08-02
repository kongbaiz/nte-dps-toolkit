import type { editor } from "monaco-editor/editor/editor.api";

export const MOD_SOURCE_SUGGEST_OPTIONS = {
  quickSuggestions: {
    other: true,
    comments: false,
    strings: false,
  },
  quickSuggestionsDelay: 75,
  suggestOnTriggerCharacters: true,
  wordBasedSuggestions: "off",
} satisfies Pick<
  editor.IStandaloneEditorConstructionOptions,
  | "quickSuggestions"
  | "quickSuggestionsDelay"
  | "suggestOnTriggerCharacters"
  | "wordBasedSuggestions"
>;

export const MOD_SOURCE_COMPLETION_TRIGGER_CHARACTERS = [":", "."];
