import {
  Braces,
  ChevronLeft,
  ChevronRight,
  ChevronsUpDown,
  Columns3,
  Database,
  KeyRound,
  List,
  PanelRightClose,
  Search,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { CollectionOverview, FieldDef } from "@stellardb/client";

function fieldMatches(field: FieldDef, query: string): boolean {
  return field.name.toLowerCase().includes(query) ||
    Boolean(field.fields?.some((nested) => fieldMatches(nested, query)));
}

function FieldNode({
  field,
  prefix,
  expanded,
  onToggle,
  onInsert,
}: {
  field: FieldDef;
  prefix: string;
  expanded: Set<string>;
  onToggle: (key: string) => void;
  onInsert: (text: string) => void;
}) {
  const path = prefix ? `${prefix}.${field.name}` : field.name;
  const nested = Boolean(field.fields?.length);
  const open = expanded.has(`field:${path}`);
  return (
    <div className="schema-node">
      <button
        className="schema-field"
        onClick={() => nested ? onToggle(`field:${path}`) : onInsert(path)}
        onDoubleClick={() => onInsert(path)}
        title={`Insert ${path}`}
      >
        {nested ? <ChevronRight size={11} className={open ? "rotate-90" : ""} /> : <Columns3 size={11} />}
        {nested && (field.type.startsWith("[") ? <List size={11} /> : <Braces size={11} />)}
        <span>{field.name}</span>
        {field.required && <i>*</i>}
        <small data-type={field.type.toLowerCase()}>{field.type}</small>
      </button>
      {nested && open && (
        <div className="schema-children">
          {field.fields?.map((child) => (
            <FieldNode key={child.name} field={child} prefix={path} expanded={expanded} onToggle={onToggle} onInsert={onInsert} />
          ))}
        </div>
      )}
    </div>
  );
}

export function StationSchema({
  collections,
  collapsed,
  onCollapsedChange,
  onInsert,
}: {
  collections: CollectionOverview[];
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  onInsert: (text: string) => void;
}) {
  const [expanded, setExpanded] = useState(new Set<string>());
  const [search, setSearch] = useState("");

  const schemas = useMemo(
    () => collections.filter((collection) => collection.schema),
    [collections],
  );
  const filtered = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return schemas;
    return schemas.filter((collection) =>
      collection.name.toLowerCase().includes(query) ||
      Boolean(collection.schema?.fields.some((field) => fieldMatches(field, query))),
    );
  }, [schemas, search]);

  function toggle(key: string) {
    setExpanded((previous) => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function toggleAll() {
    const allOpen = schemas.length > 0 && schemas.every((schema) => expanded.has(schema.name));
    setExpanded(allOpen ? new Set() : new Set(schemas.map((schema) => schema.name)));
  }

  if (collapsed) {
    return (
      <aside className="schema-panel is-collapsed">
        <button onClick={() => onCollapsedChange(false)} aria-label="Expand schema" title="Expand schema">
          <ChevronLeft size={15} />
          <Database size={15} />
          <span>Schema</span>
        </button>
      </aside>
    );
  }

  return (
    <aside className="schema-panel" aria-label="Database schema">
      <header>
        <div><Database size={14} /><span>Schema</span></div>
        <div>
          <button onClick={toggleAll} aria-label="Expand or collapse all collections"><ChevronsUpDown size={13} /></button>
          <button onClick={() => onCollapsedChange(true)} aria-label="Collapse schema"><PanelRightClose size={14} /></button>
        </div>
      </header>
      {schemas.length > 2 && (
        <label className="schema-filter">
          <Search size={13} />
          <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Filter schema…" />
        </label>
      )}
      <div className="schema-tree" role="tree">
        {filtered.length === 0 ? (
          <div className="schema-empty">{schemas.length === 0 ? "No schema loaded." : "No matches."}</div>
        ) : filtered.map((collection) => {
          const open = expanded.has(collection.name);
          return (
            <div className="schema-collection" key={collection.name}>
              <button
                className="schema-collection-button"
                onClick={() => toggle(collection.name)}
                onDoubleClick={() => onInsert(collection.name)}
              >
                <ChevronRight size={12} className={open ? "rotate-90" : ""} />
                <Database size={12} />
                <span>{collection.name}</span>
                <small>{collection.schema?.fields.length ?? 0}</small>
              </button>
              {open && collection.schema && (
                <div className="schema-collection-body">
                  {collection.schema.fields.map((field) => (
                    <FieldNode key={field.name} field={field} prefix="" expanded={expanded} onToggle={toggle} onInsert={onInsert} />
                  ))}
                  {collection.schema.indexes.map((index, position) => (
                    <div className="schema-index" key={index.name || position} title={`${index.index_type} on ${index.fields.join(", ")}`}>
                      <KeyRound size={10} />
                      <span>{index.name || index.fields.join(", ")}</span>
                      <small>{index.index_type}</small>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </aside>
  );
}
