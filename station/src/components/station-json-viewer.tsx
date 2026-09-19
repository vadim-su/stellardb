import Editor, { type Monaco } from "@monaco-editor/react";
import { useMemo } from "react";
import type * as MonacoType from "monaco-editor";

const JSON_THEME_ID = "stellardb-station-json";

const jsonTheme: MonacoType.editor.IStandaloneThemeData = {
  base: "vs-dark",
  inherit: true,
  rules: [
    { token: "string.key.json", foreground: "c7a4ff" },
    { token: "string.value.json", foreground: "a9ed6f" },
    { token: "number.json", foreground: "ffb8df" },
    { token: "keyword.json", foreground: "64e5ff" },
    { token: "delimiter.bracket.json", foreground: "8d98ad" },
    { token: "delimiter.comma.json", foreground: "656f84" },
  ],
  colors: {
    "editor.background": "#00000000",
    "editor.foreground": "#e9edf5",
    "editorLineNumber.foreground": "#394356",
    "editorLineNumber.activeForeground": "#8d98ad",
    "editorGutter.background": "#00000000",
    "editor.selectionBackground": "#9c7be045",
    "editor.lineHighlightBackground": "#ffffff04",
    "scrollbar.shadow": "#00000000",
    "scrollbarSlider.background": "#9c7be020",
    "scrollbarSlider.hoverBackground": "#9c7be038",
  },
};

export function StationJsonViewer({ value, statementType }: {
  value: unknown;
  statementType: string;
}) {
  const json = useMemo(() => JSON.stringify(value, null, 2), [value]);

  function beforeMount(monaco: Monaco) {
    monaco.editor.defineTheme(JSON_THEME_ID, jsonTheme);
  }

  return (
    <div className="json-result">
      <span>{`// ${statementType}`}</span>
      <div className="station-json-viewer">
        <Editor
          height="100%"
          language="json"
          theme={JSON_THEME_ID}
          value={json}
          beforeMount={beforeMount}
          options={{
            readOnly: true,
            domReadOnly: true,
            automaticLayout: true,
            minimap: { enabled: false },
            lineNumbers: "on",
            lineNumbersMinChars: 3,
            glyphMargin: false,
            folding: true,
            fontFamily: "'DM Mono', 'SF Mono', Menlo, monospace",
            fontSize: 11,
            lineHeight: 19,
            padding: { top: 8, bottom: 12 },
            renderLineHighlight: "none",
            scrollBeyondLastLine: false,
            wordWrap: "on",
            overviewRulerBorder: false,
            overviewRulerLanes: 0,
            hideCursorInOverviewRuler: true,
            scrollbar: { verticalScrollbarSize: 6, horizontalScrollbarSize: 6 },
          }}
        />
      </div>
    </div>
  );
}
