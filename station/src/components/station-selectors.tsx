import {
  Check,
  ChevronDown,
  Database,
  Pencil,
  Plus,
  Server,
  Trash2,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { StationConnection } from "../lib/station-api";

function useOutsideClose(open: boolean, onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [open, onClose]);
  return ref;
}

export interface StationConnectionRecord extends StationConnection {
  id: string;
  name: string;
  displayUrl?: string;
}

export function ConnectionMenu({
  connections,
  activeId,
  onSelect,
  onAdd,
  onEdit,
  onRemove,
}: {
  connections: StationConnectionRecord[];
  activeId: string;
  onSelect: (connection: StationConnectionRecord) => void;
  onAdd: () => void;
  onEdit: (connection: StationConnectionRecord) => void;
  onRemove: (connection: StationConnectionRecord) => void;
}) {
  const [open, setOpen] = useState(false);
  const active = connections.find((connection) => connection.id === activeId);
  const ref = useOutsideClose(open, () => setOpen(false));
  return (
    <div className="selector-root connection-selector" ref={ref}>
      <button className="selector-trigger" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        <Server size={14} />
        <span>{active?.name ?? "No connection"}</span>
        <ChevronDown size={13} />
      </button>
      {open && (
        <div className="selector-menu connection-menu" role="menu">
          <header><span>Connections</span><small>{connections.length}</small></header>
          <div className="connection-menu-list">
            {connections.map((connection) => (
              <div className="connection-menu-row" key={connection.id}>
                <button onClick={() => { onSelect(connection); setOpen(false); }}>
                  <Check size={13} className={connection.id === activeId ? "is-visible" : ""} />
                  <span><strong>{connection.name}</strong><small>{connection.displayUrl ?? connection.baseUrl}</small></span>
                </button>
                <button onClick={() => { onEdit(connection); setOpen(false); }} aria-label={`Edit ${connection.name}`}><Pencil size={13} /></button>
                <button onClick={() => onRemove(connection)} aria-label={`Delete ${connection.name}`}><Trash2 size={13} /></button>
              </div>
            ))}
          </div>
          <button className="selector-add" onClick={() => { onAdd(); setOpen(false); }}><Plus size={13} /> Add connection</button>
        </div>
      )}
    </div>
  );
}

export function DatabaseMenu({
  databases,
  value,
  onChange,
}: {
  databases: string[];
  value: string;
  onChange: (database: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useOutsideClose(open, () => setOpen(false));
  return (
    <div className="selector-root database-selector" ref={ref}>
      <button className="selector-trigger" onClick={() => setOpen((current) => !current)} aria-expanded={open}>
        <Database size={14} />
        <span>{value || "No database"}</span>
        <ChevronDown size={13} />
      </button>
      {open && (
        <div className="selector-menu database-menu" role="listbox" aria-label="Databases">
          <header><span>Databases</span><small>{databases.length}</small></header>
          {databases.map((database) => (
            <button key={database} className={database === value ? "is-active" : ""} onClick={() => { onChange(database); setOpen(false); }}>
              <Database size={13} /><span>{database}</span><Check size={13} />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
