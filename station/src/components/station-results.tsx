import {
  Braces,
  Check,
  CircleAlert,
  Clock3,
  Copy,
  Download,
  LoaderCircle,
  Network,
  Play,
  Square,
  Table2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { StationQueryResult } from "../lib/station-api";
import { StationGraph } from "./station-graph";
import { StationJsonViewer } from "./station-json-viewer";

type ResultMode = "table" | "json" | "graph";

function columnsFor(rows: Record<string, unknown>[]) {
  const keys = rows[0] ? Object.keys(rows[0]) : [];
  return keys.includes("id") ? ["id", ...keys.filter((key) => key !== "id")] : keys;
}

function stringify(value: unknown) {
  if (value === null) return "NULL";
  if (value === undefined) return "—";
  if (typeof value === "object" && value && "$type" in value && "value" in value) {
    return String((value as { value: unknown }).value);
  }
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}

export function StationResults({
  result,
  error,
  running,
  canRun,
  onRun,
  onCancel,
}: {
  result: StationQueryResult | null;
  error: string | null;
  running: boolean;
  canRun: boolean;
  onRun: () => void;
  onCancel: () => void;
}) {
  const [mode, setMode] = useState<ResultMode>("table");
  const [statementIndex, setStatementIndex] = useState(0);
  const [detailRow, setDetailRow] = useState<Record<string, unknown> | null>(null);

  useEffect(() => {
    if (!result) return;
    setStatementIndex(Math.max(0, result.allResults.length - 1));
    setMode("table");
  }, [result]);

  const statement = result?.allResults[statementIndex];
  const rows = statement?.data ?? result?.rows ?? [];
  const columns = useMemo(() => columnsFor(rows), [rows]);

  async function copyJson() {
    await navigator.clipboard.writeText(JSON.stringify(rows, null, 2));
  }

  function downloadDelimited(
    delimiter: "," | "\t",
    extension: "csv" | "tsv",
    mimeType: string,
  ) {
    const outputRows = [columns.join(delimiter), ...rows.map((row) => columns.map((column) => {
      const value = stringify(row[column]);
      const needsQuotes = value.includes(delimiter) || /["\r\n]/.test(value);
      return needsQuotes ? `"${value.replace(/"/g, '""')}"` : value;
    }).join(delimiter))];
    const url = URL.createObjectURL(new Blob([outputRows.join("\r\n")], { type: `${mimeType};charset=utf-8` }));
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `station-results.${extension}`;
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);
  }

  return (
    <section className="results-panel station-results">
      <header className="results-toolbar">
        <div className="result-run-controls">
          <button onClick={running ? onCancel : onRun} disabled={!running && !canRun} aria-label={running ? "Cancel query" : "Run query"}>
            {running ? <Square size={14} /> : <Play size={14} fill="currentColor" />}
          </button>
        </div>
        <span className="toolbar-divider" />
        <div className="result-modes" role="tablist" aria-label="Result view">
          <button className={mode === "table" ? "is-active" : ""} onClick={() => setMode("table")}><Table2 size={14} /> Table</button>
          <button className={mode === "json" ? "is-active" : ""} onClick={() => setMode("json")}><Braces size={14} /> JSON</button>
          <button className={mode === "graph" ? "is-active" : ""} onClick={() => setMode("graph")}><Network size={14} /> Graph</button>
        </div>
        {result && rows.length > 0 && (
          <div className="result-export-actions">
            <button onClick={() => void copyJson()}><Copy size={13} /> Copy JSON</button>
            <button onClick={() => downloadDelimited(",", "csv", "text/csv")} title="Download query results as CSV"><Download size={13} /> CSV</button>
            <button onClick={() => downloadDelimited("\t", "tsv", "text/tab-separated-values")} title="Download query results as TSV"><Download size={13} /> TSV</button>
          </div>
        )}
        {result && (
          <span className="result-meta"><Check size={13} /> {rows.length.toLocaleString()} rows <i /> <Clock3 size={13} /> {result.elapsedMs.toFixed(2)} ms</span>
        )}
        {running && <span className="result-meta"><LoaderCircle size={13} className="spin" /> Executing</span>}
      </header>

      {result && result.allResults.length > 1 && (
        <div className="statement-tabs" role="tablist" aria-label="Statements">
          {result.allResults.map((item, index) => (
            <button key={index} className={statementIndex === index ? "is-active" : ""} onClick={() => { setStatementIndex(index); setMode("table"); }}>
              {item.statementType} <span>{item.data.length}</span>
            </button>
          ))}
        </div>
      )}

      <div className="result-content">
        {running ? (
          <div className="results-empty"><LoaderCircle size={22} className="spin" /><strong>Executing query</strong></div>
        ) : error ? (
          <div className="query-error" role="alert">
            <CircleAlert size={19} />
            <div><strong>Query failed</strong><p>{error}</p></div>
          </div>
        ) : !result ? (
          <div className="results-empty"><div><Table2 size={22} /></div><strong>No results yet</strong><span>Press ⌘ Enter to execute the active query.</span></div>
        ) : rows.length === 0 ? (
          <div className="results-empty success"><div><Check size={22} /></div><strong>{statement?.statementType ?? result.statementType} completed</strong><span>{result.rowCount.toLocaleString()} rows affected.</span></div>
        ) : mode === "graph" ? (
          <StationGraph result={{ ...result, allResults: statement ? [statement] : result.allResults }} />
        ) : mode === "json" ? (
          <StationJsonViewer value={rows} statementType={statement?.statementType ?? result.statementType} />
        ) : (
          <div className="result-table-wrap">
            <table className="result-table">
              <thead><tr><th className="row-number">#</th>{columns.map((column) => <th key={column}>{column}</th>)}</tr></thead>
              <tbody>{rows.map((row, index) => (
                <tr key={index} onClick={() => setDetailRow(row)}>
                  <td className="row-number">{index + 1}</td>
                  {columns.map((column) => <td key={column} title={stringify(row[column])}><span className={row[column] === null ? "null-value" : ""}>{stringify(row[column])}</span></td>)}
                </tr>
              ))}</tbody>
            </table>
          </div>
        )}
      </div>

      {detailRow && (
        <aside className="row-detail" aria-label="Row details">
          <header><div><small>ROW DETAIL</small><strong>{typeof detailRow.id === "string" ? detailRow.id : "Selected row"}</strong></div><button onClick={() => setDetailRow(null)} aria-label="Close row details"><X size={15} /></button></header>
          <div>{Object.entries(detailRow).map(([key, value]) => (
            <section key={key}><span>{key}</span><button onClick={() => void navigator.clipboard.writeText(stringify(value))}><Copy size={12} /></button><pre>{stringify(value)}</pre></section>
          ))}</div>
        </aside>
      )}
    </section>
  );
}
