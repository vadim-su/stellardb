import Editor, { type Monaco, type OnMount } from "@monaco-editor/react";
import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";
import type * as MonacoType from "monaco-editor";
import type { CollectionSchema, FieldDef } from "@stellardb/client";

const LANGUAGE_ID = "stellarsql";
const THEME_ID = "stellardb-station";
const QUERY_MARKER_OWNER = "stellardb-query";

const KEYWORDS = [
  "SELECT", "FROM", "WHERE", "ORDER", "BY", "ASC", "DESC", "LIMIT",
  "OFFSET", "GROUP", "DISTINCT", "VALUE", "FETCH", "AS", "IN", "AND",
  "OR", "NOT", "IS", "MATCH", "MODE", "INSERT", "INTO", "CREATE",
  "UPDATE", "SET", "DELETE", "LET", "RELATE", "CONTENT", "RETURN",
  "AFTER", "BEFORE", "DEFINE", "DROP", "COLLECTION", "COLLECTIONS",
  "DESCRIBE", "INDEX", "ON", "UNIQUE", "FULLTEXT", "REINDEX", "SCHEMA",
  "STRICT", "FLEXIBLE", "REQUIRED", "DEFAULT", "CASCADE", "HNSW",
  "DIMENSION", "DIST", "COSINE", "EUCLIDEAN", "DOT", "ANALYZER",
  "TOKENIZER", "FILTERS", "BEGIN", "COMMIT", "ROLLBACK", "IF", "THEN",
  "ELSE", "END", "FOR", "DO", "BREAK", "CONTINUE", "EXPLAIN", "ANALYZE",
  "ALL", "TRUE", "FALSE", "NULL", "NONE",
];

const FUNCTIONS = [
  "COUNT", "SUM", "AVG", "MIN", "MAX", "UPPER", "LOWER", "LENGTH",
  "TRIM", "CONCAT", "SUBSTRING", "REPLACE", "CONTAINS", "STARTS_WITH",
  "ENDS_WITH", "ABS", "ROUND", "CEIL", "FLOOR", "TYPE", "TO_INT",
  "TO_FLOAT", "TO_STRING", "TO_BOOL", "ARRAY_LENGTH", "ARRAY_PUSH",
  "ARRAY_CONTAINS", "KEYS", "VALUES", "ID", "NOW", "VECTOR_DISTANCE",
];

const languageConfiguration: MonacoType.languages.LanguageConfiguration = {
  comments: { lineComment: "//", blockComment: ["/*", "*/"] },
  brackets: [["(", ")"], ["[", "]"], ["{", "}"]],
  autoClosingPairs: [
    { open: "(", close: ")" },
    { open: "[", close: "]" },
    { open: "{", close: "}" },
    { open: "'", close: "'", notIn: ["string", "comment"] },
    { open: '"', close: '"', notIn: ["string", "comment"] },
  ],
};

const monarchTokens: MonacoType.languages.IMonarchLanguage = {
  ignoreCase: true,
  keywords: KEYWORDS,
  functions: FUNCTIONS,
  tokenizer: {
    root: [
      [/\/\/.*$/, "comment"],
      [/\/\*/, "comment", "@comment"],
      [/'/, "string", "@singleString"],
      [/"/, "string", "@doubleString"],
      [/\d+(\.\d+)?/, "number"],
      [/[a-zA-Z_]\w*/, { cases: { "@keywords": "keyword", "@functions": "function", "@default": "identifier" } }],
      [/[<>=!]+|[+\-*/%]/, "operator"],
      [/[()[\]{},;.]/, "delimiter"],
      [/\s+/, "white"],
    ],
    comment: [[/[^/*]+/, "comment"], [/\*\//, "comment", "@pop"], [/./, "comment"]],
    singleString: [[/[^']+/, "string"], [/''/, "string"], [/'/, "string", "@pop"]],
    doubleString: [[/[^\"]+/, "string"], [/""/, "string"], [/"/, "string", "@pop"]],
  },
};

const theme: MonacoType.editor.IStandaloneThemeData = {
  base: "vs-dark",
  inherit: true,
  rules: [
    { token: "keyword", foreground: "c7a4ff", fontStyle: "bold" },
    { token: "function", foreground: "72ddff" },
    { token: "operator", foreground: "a99bc3" },
    { token: "string", foreground: "a9ed6f" },
    { token: "number", foreground: "ffb8df" },
    { token: "comment", foreground: "656f84", fontStyle: "italic" },
    { token: "identifier", foreground: "e9edf5" },
    { token: "delimiter", foreground: "7d879a" },
  ],
  colors: {
    "editor.background": "#00000000",
    "editor.foreground": "#e9edf5",
    "editor.selectionBackground": "#9c7be045",
    "editor.lineHighlightBackground": "#ffffff06",
    "editorCursor.foreground": "#c8ff68",
    "editorLineNumber.foreground": "#394356",
    "editorLineNumber.activeForeground": "#8d98ad",
    "editorGutter.background": "#00000000",
    "editorSuggestWidget.background": "#121825f5",
    "editorSuggestWidget.border": "#9c7be030",
    "editorSuggestWidget.selectedBackground": "#9c7be025",
    "editorSuggestWidget.highlightForeground": "#c7a4ff",
    "scrollbar.shadow": "#00000000",
    "scrollbarSlider.background": "#9c7be020",
    "scrollbarSlider.hoverBackground": "#9c7be038",
  },
};

function flattenFields(fields: FieldDef[], prefix = ""): string[] {
  return fields.flatMap((field) => {
    const path = prefix ? `${prefix}.${field.name}` : field.name;
    return [path, ...(field.fields ? flattenFields(field.fields, path) : [])];
  });
}

function completionProvider(
  monaco: Monaco,
  schemas: Map<string, CollectionSchema>,
): MonacoType.languages.CompletionItemProvider {
  return {
    triggerCharacters: [" ", "."],
    provideCompletionItems(model, position) {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endLineNumber: position.lineNumber,
        endColumn: word.endColumn,
      };
      const suggestions: MonacoType.languages.CompletionItem[] = [];
      for (const keyword of KEYWORDS) {
        suggestions.push({ label: keyword, kind: monaco.languages.CompletionItemKind.Keyword, insertText: keyword, range, sortText: `3_${keyword}` });
      }
      for (const fn of FUNCTIONS) {
        suggestions.push({ label: fn, kind: monaco.languages.CompletionItemKind.Function, insertText: `${fn}($0)`, insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, range, sortText: `2_${fn}` });
      }
      for (const [collection, schema] of schemas) {
        suggestions.push({ label: collection, kind: monaco.languages.CompletionItemKind.Class, insertText: collection, detail: "collection", range, sortText: `0_${collection}` });
        for (const field of flattenFields(schema.fields)) {
          suggestions.push({ label: field, kind: monaco.languages.CompletionItemKind.Field, insertText: field, detail: collection, range, sortText: `1_${field}` });
        }
      }
      return { suggestions };
    },
  };
}

interface QueryDiagnostic {
  line: number;
  column: number;
  message: string;
}

function queryDiagnostic(error: string | null): QueryDiagnostic | null {
  if (!error) return null;
  const location = /-->\s*(\d+)\s*:\s*(\d+)/.exec(error);
  if (!location) return null;

  const title = error
    .slice(0, location.index)
    .trim()
    .replace(/:\s*$/, "") || "Query error";
  const detail = /\|\s*=\s*([\s\S]+)$/.exec(error)?.[1]
    ?.replace(/\s+/g, " ")
    .trim();

  return {
    line: Number(location[1]),
    column: Number(location[2]),
    message: detail ? `${title}: ${detail}` : error.replace(/\s+/g, " ").trim(),
  };
}

function applyQueryDiagnostic(
  editor: MonacoType.editor.IStandaloneCodeEditor,
  monaco: Monaco,
  error: string | null,
) {
  const model = editor.getModel();
  if (!model) return;
  const diagnostic = queryDiagnostic(error);
  if (!diagnostic) {
    monaco.editor.setModelMarkers(model, QUERY_MARKER_OWNER, []);
    return;
  }

  const lineNumber = Math.min(Math.max(diagnostic.line, 1), model.getLineCount());
  const maxColumn = model.getLineMaxColumn(lineNumber);
  const column = Math.min(Math.max(diagnostic.column, 1), maxColumn);
  const word = model.getWordAtPosition({ lineNumber, column });
  const startColumn = word?.startColumn ?? Math.min(column, Math.max(1, maxColumn - 1));
  const endColumn = word?.endColumn ?? Math.min(maxColumn, startColumn + 1);

  monaco.editor.setModelMarkers(model, QUERY_MARKER_OWNER, [{
    severity: monaco.MarkerSeverity.Error,
    message: diagnostic.message,
    source: "StellarDB",
    startLineNumber: lineNumber,
    startColumn,
    endLineNumber: lineNumber,
    endColumn,
  }]);
  editor.revealPositionInCenterIfOutsideViewport({ lineNumber, column });
}

export interface StationEditorHandle {
  insertAtCursor: (text: string) => void;
  selectedText: () => string | undefined;
}

interface StationEditorProps {
  value: string;
  error: string | null;
  schemas: Map<string, CollectionSchema>;
  onChange: (value: string) => void;
  onExecute: (sql?: string) => void;
  onCursorChange: (line: number, column: number) => void;
}

export const StationEditor = forwardRef<StationEditorHandle, StationEditorProps>(
  function StationEditor({ value, error, schemas, onChange, onExecute, onCursorChange }, ref) {
    const editorRef = useRef<MonacoType.editor.IStandaloneCodeEditor | null>(null);
    const monacoRef = useRef<Monaco | null>(null);
    const completionRef = useRef<MonacoType.IDisposable | null>(null);
    const onChangeRef = useRef(onChange);
    const onExecuteRef = useRef(onExecute);
    const errorRef = useRef(error);
    const suppressRef = useRef(false);
    onChangeRef.current = onChange;
    onExecuteRef.current = onExecute;
    errorRef.current = error;

    useImperativeHandle(ref, () => ({
      insertAtCursor(text) {
        const editor = editorRef.current;
        const selection = editor?.getSelection();
        if (!editor || !selection) return;
        editor.executeEdits("station-schema", [{ range: selection, text, forceMoveMarkers: true }]);
        editor.focus();
      },
      selectedText() {
        const editor = editorRef.current;
        const selection = editor?.getSelection();
        if (!editor || !selection || selection.isEmpty()) return undefined;
        return editor.getModel()?.getValueInRange(selection) || undefined;
      },
    }));

    useEffect(() => {
      const editor = editorRef.current;
      if (!editor || editor.getValue() === value) return;
      suppressRef.current = true;
      editor.setValue(value);
      suppressRef.current = false;
    }, [value]);

    useEffect(() => {
      if (!monacoRef.current) return;
      completionRef.current?.dispose();
      completionRef.current = monacoRef.current.languages.registerCompletionItemProvider(
        LANGUAGE_ID,
        completionProvider(monacoRef.current, schemas),
      );
      return () => completionRef.current?.dispose();
    }, [schemas]);

    useEffect(() => {
      const editor = editorRef.current;
      const monaco = monacoRef.current;
      if (editor && monaco) applyQueryDiagnostic(editor, monaco, error);
    }, [error]);

    function beforeMount(monaco: Monaco) {
      if (!monaco.languages.getLanguages().some((language: MonacoType.languages.ILanguageExtensionPoint) => language.id === LANGUAGE_ID)) {
        monaco.languages.register({ id: LANGUAGE_ID });
        monaco.languages.setMonarchTokensProvider(LANGUAGE_ID, monarchTokens);
        monaco.languages.setLanguageConfiguration(LANGUAGE_ID, languageConfiguration);
      }
      monaco.editor.defineTheme(THEME_ID, theme);
    }

    const onMount: OnMount = (editor, monaco) => {
      editorRef.current = editor;
      monacoRef.current = monaco;
      editor.onDidChangeModelContent(() => {
        if (!suppressRef.current) onChangeRef.current(editor.getValue());
      });
      editor.onDidChangeCursorPosition((event) => onCursorChange(event.position.lineNumber, event.position.column));
      editor.addAction({
        id: "station-run-query",
        label: "Run query",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
        run(ed) {
          const selection = ed.getSelection();
          const selected = selection && !selection.isEmpty() ? ed.getModel()?.getValueInRange(selection) : undefined;
          onExecuteRef.current(selected || undefined);
        },
      });
      editor.addAction({ id: "station-run-query-f5", label: "Run query (F5)", keybindings: [monaco.KeyCode.F5], run: () => onExecuteRef.current() });
      completionRef.current?.dispose();
      completionRef.current = monaco.languages.registerCompletionItemProvider(LANGUAGE_ID, completionProvider(monaco, schemas));
      applyQueryDiagnostic(editor, monaco, errorRef.current);
    };

    return (
      <Editor
        height="100%"
        language={LANGUAGE_ID}
        theme={THEME_ID}
        value={value}
        beforeMount={beforeMount}
        onMount={onMount}
        options={{
          minimap: { enabled: false },
          lineNumbers: "on",
          fontFamily: "'DM Mono', 'SF Mono', Menlo, monospace",
          fontSize: 13,
          lineHeight: 22,
          padding: { top: 14 },
          scrollBeyondLastLine: false,
          automaticLayout: true,
          tabSize: 2,
          suggestOnTriggerCharacters: true,
          wordWrap: "off",
          fixedOverflowWidgets: true,
          hover: { above: false },
          overviewRulerBorder: false,
          overviewRulerLanes: 0,
          hideCursorInOverviewRuler: true,
          renderLineHighlight: "line",
          renderValidationDecorations: "on",
          scrollbar: { verticalScrollbarSize: 6, horizontalScrollbarSize: 6 },
        }}
      />
    );
  },
);
