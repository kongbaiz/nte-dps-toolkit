import { useEffect, useRef } from "react";
import "monaco-editor/editor/browser/coreCommands";
import "monaco-editor/editor/contrib/bracketMatching/browser/bracketMatching";
import "monaco-editor/editor/contrib/caretOperations/browser/caretOperations";
import "monaco-editor/editor/contrib/clipboard/browser/clipboard";
import "monaco-editor/editor/contrib/comment/browser/comment";
import "monaco-editor/editor/contrib/contextmenu/browser/contextmenu";
import "monaco-editor/editor/contrib/cursorUndo/browser/cursorUndo";
import "monaco-editor/editor/contrib/dnd/browser/dnd";
import "monaco-editor/editor/contrib/find/browser/findController";
import "monaco-editor/editor/contrib/folding/browser/folding";
import "monaco-editor/editor/contrib/hover/browser/hoverContribution";
import "monaco-editor/editor/contrib/indentation/browser/indentation";
import "monaco-editor/editor/contrib/lineSelection/browser/lineSelection";
import "monaco-editor/editor/contrib/linesOperations/browser/linesOperations";
import "monaco-editor/editor/contrib/multicursor/browser/multicursor";
import "monaco-editor/editor/contrib/parameterHints/browser/parameterHints";
import "monaco-editor/editor/contrib/semanticTokens/browser/documentSemanticTokens";
import "monaco-editor/editor/contrib/semanticTokens/browser/viewportSemanticTokens";
import "monaco-editor/editor/contrib/smartSelect/browser/smartSelect";
import "monaco-editor/editor/contrib/snippet/browser/snippetController2";
import "monaco-editor/editor/contrib/suggest/browser/suggestController";
import "monaco-editor/editor/contrib/wordHighlighter/browser/wordHighlighter";
import "monaco-editor/editor/contrib/wordOperations/browser/wordOperations";
import "monaco-editor/editor/contrib/wordPartOperations/browser/wordPartOperations";
import * as monaco from "monaco-editor/editor/editor.api";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import "monaco-editor/languages/definitions/cpp/register";

import { t, tf } from "@/lib/i18n";
import type { ModStudioSdkSchemaSnapshot } from "@/lib/tauri/mod-studio-contract";

import {
  modSourceCompletion,
  modSourceHover,
  modSourceOccurrences,
  modSourceSignatureHelp,
  type ModSourceCompletionKind,
} from "./mod-source-intelligence";
import {
  MOD_SOURCE_COMPLETION_TRIGGER_CHARACTERS,
  MOD_SOURCE_SUGGEST_OPTIONS,
} from "./mod-source-editor-options";
import {
  MOD_SOURCE_SEMANTIC_TOKEN_TYPES,
  modSourceSemanticTokens,
  type ModSourceSemanticToken,
} from "./mod-source-semantic";

globalThis.MonacoEnvironment = {
  getWorker: () => new EditorWorker(),
};

const LIGHT_EDITOR_THEME = "nte-vs";
const DARK_EDITOR_THEME = "nte-vs-dark";

monaco.editor.defineTheme(LIGHT_EDITOR_THEME, {
  base: "vs",
  inherit: true,
  colors: {},
  rules: [
    { token: "macro", foreground: "AF00DB" },
    { token: "namespace", foreground: "267F99" },
    { token: "type", foreground: "267F99" },
    { token: "function", foreground: "795E26" },
    { token: "variable", foreground: "001080" },
    { token: "property", foreground: "001080" },
  ],
});

monaco.editor.defineTheme(DARK_EDITOR_THEME, {
  base: "vs-dark",
  inherit: true,
  colors: {},
  rules: [
    { token: "macro", foreground: "C586C0" },
    { token: "namespace", foreground: "4EC9B0" },
    { token: "type", foreground: "4EC9B0" },
    { token: "function", foreground: "DCDCAA" },
    { token: "variable", foreground: "9CDCFE" },
    { token: "property", foreground: "9CDCFE" },
  ],
});

export interface ModSourceCursor {
  line: number;
  column: number;
}

interface ModSourceEditorProps {
  source: string;
  diagnosticLine: number | null;
  label: string;
  sdkSchema: ModStudioSdkSchemaSnapshot | null;
  theme: "light" | "dark";
  onChange: (source: string) => void;
  onCursorChange: (cursor: ModSourceCursor) => void;
  onSave: () => void;
}

export function ModSourceEditor({
  source,
  diagnosticLine,
  label,
  sdkSchema,
  theme,
  onChange,
  onCursorChange,
  onSave,
}: ModSourceEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor>(null);
  const modelRef = useRef<monaco.editor.ITextModel>(null);
  const semanticChangeEmitterRef = useRef<monaco.Emitter<void>>(null);
  const applyingExternalSourceRef = useRef(false);
  const sourceRef = useRef(source);
  const schemaRef = useRef(sdkSchema);
  const labelRef = useRef(label);
  const themeRef = useRef(theme);
  const onChangeRef = useRef(onChange);
  const onCursorChangeRef = useRef(onCursorChange);
  const onSaveRef = useRef(onSave);

  sourceRef.current = source;
  schemaRef.current = sdkSchema;
  labelRef.current = label;
  themeRef.current = theme;
  onChangeRef.current = onChange;
  onCursorChangeRef.current = onCursorChange;
  onSaveRef.current = onSave;

  useEffect(() => {
    const container = containerRef.current;
    if (container === null) {
      return;
    }

    const model = monaco.editor.createModel(sourceRef.current, "cpp");
    const semanticChangeEmitter = new monaco.Emitter<void>();
    semanticChangeEmitterRef.current = semanticChangeEmitter;
    const editor = monaco.editor.create(container, {
      model,
      theme:
        themeRef.current === "dark" ? DARK_EDITOR_THEME : LIGHT_EDITOR_THEME,
      ariaLabel: labelRef.current,
      automaticLayout: true,
      fontFamily: 'Consolas, "Courier New", monospace',
      fontSize: 14,
      lineHeight: 20,
      tabSize: 4,
      insertSpaces: true,
      detectIndentation: false,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      renderLineHighlight: "all",
      roundedSelection: false,
      selectionHighlight: true,
      occurrencesHighlight: "singleFile",
      occurrencesHighlightDelay: 250,
      hover: {
        enabled: "on",
        delay: 300,
        sticky: true,
        above: false,
      },
      ...MOD_SOURCE_SUGGEST_OPTIONS,
      "semanticHighlighting.enabled": true,
      parameterHints: {
        enabled: true,
        cycle: true,
      },
      snippetSuggestions: "inline",
      lineNumbersMinChars: 4,
      glyphMargin: false,
      folding: true,
      links: false,
      contextmenu: true,
      fixedOverflowWidgets: true,
      overviewRulerLanes: 0,
      overviewRulerBorder: false,
      hideCursorInOverviewRuler: true,
      padding: { top: 8, bottom: 8 },
    });
    editorRef.current = editor;
    modelRef.current = model;
    onCursorChangeRef.current({ line: 1, column: 1 });

    const contentSubscription = model.onDidChangeContent(() => {
      if (!applyingExternalSourceRef.current) {
        onChangeRef.current(model.getValue());
      }
    });
    const cursorSubscription = editor.onDidChangeCursorPosition(
      ({ position }) =>
        onCursorChangeRef.current({
          line: position.lineNumber,
          column: position.column,
        }),
    );
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () =>
      onSaveRef.current(),
    );

    const completionProvider = monaco.languages.registerCompletionItemProvider(
      "cpp",
      {
        triggerCharacters: MOD_SOURCE_COMPLETION_TRIGGER_CHARACTERS,
        provideCompletionItems(candidateModel, position) {
          const schema = schemaRef.current;
          if (candidateModel !== model || schema === null) {
            return { suggestions: [] };
          }
          const result = modSourceCompletion(
            schema,
            model.getValue(),
            model.getOffsetAt(position),
            true,
          );
          if (result === null) {
            return { suggestions: [] };
          }
          const start = model.getPositionAt(result.rangeStart);
          const end = model.getPositionAt(result.rangeEnd);
          const range = new monaco.Range(
            start.lineNumber,
            start.column,
            end.lineNumber,
            end.column,
          );
          return {
            suggestions: result.items.map((item, index) => ({
              label: item.label,
              kind: completionKind(item.kind),
              detail: completionDetail(item),
              documentation: {
                value: t(item.documentationKey),
              },
              filterText: item.label,
              sortText: index.toString().padStart(4, "0"),
              insertText: item.insertText,
              insertTextRules:
                item.kind === "snippet"
                  ? monaco.languages.CompletionItemInsertTextRule.KeepWhitespace
                  : undefined,
              range,
            })),
          };
        },
      },
    );

    const signatureProvider = monaco.languages.registerSignatureHelpProvider(
      "cpp",
      {
        signatureHelpTriggerCharacters: ["(", ","],
        signatureHelpRetriggerCharacters: [","],
        provideSignatureHelp(candidateModel, position) {
          const schema = schemaRef.current;
          if (candidateModel !== model || schema === null) {
            return null;
          }
          const signature = modSourceSignatureHelp(
            schema,
            model.getValue(),
            model.getOffsetAt(position),
          );
          if (signature === null) {
            return null;
          }
          return {
            value: {
              signatures: [
                {
                  label: signature.label,
                  documentation: t(signature.documentationKey),
                  parameters: signature.parameters.map((parameter) => ({
                    label: parameter,
                  })),
                  activeParameter: signature.activeParameter,
                },
              ],
              activeSignature: 0,
              activeParameter: signature.activeParameter,
            },
            dispose() {},
          };
        },
      },
    );

    const hoverProvider = monaco.languages.registerHoverProvider("cpp", {
      provideHover(candidateModel, position) {
        const schema = schemaRef.current;
        if (candidateModel !== model || schema === null) {
          return null;
        }
        const hover = modSourceHover(
          schema,
          model.getValue(),
          model.getOffsetAt(position),
        );
        if (hover === null) {
          return null;
        }
        const start = model.getPositionAt(hover.rangeStart);
        const end = model.getPositionAt(hover.rangeEnd);
        return {
          range: new monaco.Range(
            start.lineNumber,
            start.column,
            end.lineNumber,
            end.column,
          ),
          contents: [
            {
              value: `\`\`\`cpp\n${hover.detail}\n\`\`\``,
            },
            {
              value: t(hover.documentationKey),
            },
          ],
        };
      },
    });

    const documentHighlightProvider =
      monaco.languages.registerDocumentHighlightProvider("cpp", {
        provideDocumentHighlights(candidateModel, position) {
          const schema = schemaRef.current;
          if (candidateModel !== model || schema === null) {
            return null;
          }
          return modSourceOccurrences(
            schema,
            model.getValue(),
            model.getOffsetAt(position),
          ).map(({ rangeStart, rangeEnd }) => {
            const start = model.getPositionAt(rangeStart);
            const end = model.getPositionAt(rangeEnd);
            return {
              range: new monaco.Range(
                start.lineNumber,
                start.column,
                end.lineNumber,
                end.column,
              ),
              kind: monaco.languages.DocumentHighlightKind.Text,
            };
          });
        },
      });

    const semanticTokensProvider =
      monaco.languages.registerDocumentSemanticTokensProvider("cpp", {
        onDidChange: semanticChangeEmitter.event,
        getLegend: () => ({
          tokenTypes: [...MOD_SOURCE_SEMANTIC_TOKEN_TYPES],
          tokenModifiers: [],
        }),
        provideDocumentSemanticTokens(candidateModel) {
          if (candidateModel !== model) {
            return null;
          }
          return {
            data: encodeSemanticTokens(
              modSourceSemanticTokens(model.getValue(), schemaRef.current),
            ),
          };
        },
        releaseDocumentSemanticTokens() {},
      });

    return () => {
      semanticTokensProvider.dispose();
      documentHighlightProvider.dispose();
      hoverProvider.dispose();
      signatureProvider.dispose();
      completionProvider.dispose();
      cursorSubscription.dispose();
      contentSubscription.dispose();
      monaco.editor.setModelMarkers(model, "nte-mod-studio", []);
      editor.dispose();
      model.dispose();
      semanticChangeEmitter.dispose();
      editorRef.current = null;
      modelRef.current = null;
      semanticChangeEmitterRef.current = null;
    };
  }, []);

  useEffect(() => {
    const model = modelRef.current;
    if (model === null || model.getValue() === source) {
      return;
    }
    applyingExternalSourceRef.current = true;
    model.setValue(source);
    applyingExternalSourceRef.current = false;
  }, [source]);

  useEffect(() => {
    monaco.editor.setTheme(
      theme === "dark" ? DARK_EDITOR_THEME : LIGHT_EDITOR_THEME,
    );
  }, [theme]);

  useEffect(() => {
    semanticChangeEmitterRef.current?.fire();
  }, [sdkSchema]);

  useEffect(() => {
    editorRef.current?.updateOptions({ ariaLabel: label });
  }, [label]);

  useEffect(() => {
    const model = modelRef.current;
    if (model === null) {
      return;
    }
    if (
      diagnosticLine === null ||
      diagnosticLine < 1 ||
      diagnosticLine > model.getLineCount()
    ) {
      monaco.editor.setModelMarkers(model, "nte-mod-studio", []);
      return;
    }
    monaco.editor.setModelMarkers(model, "nte-mod-studio", [
      {
        severity: monaco.MarkerSeverity.Error,
        message: tf("The NTE C++ compiler rejected line {0}.", [
          diagnosticLine.toString(),
        ]),
        startLineNumber: diagnosticLine,
        startColumn: 1,
        endLineNumber: diagnosticLine,
        endColumn: model.getLineMaxColumn(diagnosticLine),
      },
    ]);
  }, [diagnosticLine, source]);

  return (
    <div
      ref={containerRef}
      className="mod-source-monaco min-h-0 flex-1"
      data-mod-source
    />
  );
}

function completionKind(
  kind: ModSourceCompletionKind,
): monaco.languages.CompletionItemKind {
  const kinds: Record<
    ModSourceCompletionKind,
    monaco.languages.CompletionItemKind
  > = {
    declaration: monaco.languages.CompletionItemKind.Keyword,
    snippet: monaco.languages.CompletionItemKind.Snippet,
    function: monaco.languages.CompletionItemKind.Function,
    property: monaco.languages.CompletionItemKind.Property,
    variable: monaco.languages.CompletionItemKind.Variable,
  };
  return kinds[kind];
}

function completionDetail(item: {
  label: string;
  kind: ModSourceCompletionKind;
  returnType: string | null;
}): string {
  if (item.returnType === null) {
    return item.label;
  }
  return item.kind === "function"
    ? `${item.label} -> ${item.returnType}`
    : `${item.label}: ${item.returnType}`;
}

function encodeSemanticTokens(tokens: ModSourceSemanticToken[]): Uint32Array {
  const data: number[] = [];
  let previousLine = 0;
  let previousStart = 0;
  for (const token of tokens) {
    const deltaLine = token.line - previousLine;
    data.push(
      deltaLine,
      deltaLine === 0 ? token.start - previousStart : token.start,
      token.length,
      MOD_SOURCE_SEMANTIC_TOKEN_TYPES.indexOf(token.kind),
      0,
    );
    previousLine = token.line;
    previousStart = token.start;
  }
  return new Uint32Array(data);
}
