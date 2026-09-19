import type { StationQueryResult } from "./station-api";

export interface StationGraphNode {
  id: string;
  label: string;
  collection: string;
  color: string;
  fields: Record<string, unknown>;
  x?: number;
  y?: number;
}

export interface StationGraphLink {
  source: string | StationGraphNode;
  target: string | StationGraphNode;
  label: string;
}

const DOCUMENT_ID = /^[a-zA-Z_]\w*:[^\s]+$/;
const COLORS = ["#b99bff", "#64e5ff", "#c8ff68", "#ffbe62", "#ff8dbe", "#7f9cff"];

function isDocumentId(value: unknown): value is string {
  return typeof value === "string" && DOCUMENT_ID.test(value);
}

function collectionOf(id: string): string {
  const separator = id.indexOf(":");
  return separator > 0 ? id.slice(0, separator) : id;
}

function labelOf(id: string, fields: Record<string, unknown>): string {
  if (typeof fields.name === "string") return fields.name;
  if (typeof fields.label === "string") return fields.label;
  return id.slice(id.indexOf(":") + 1);
}

export function extractStationGraph(result: StationQueryResult) {
  const nodes = new Map<string, StationGraphNode>();
  const links: StationGraphLink[] = [];
  const linkKeys = new Set<string>();
  const collectionColors = new Map<string, string>();

  function colorFor(collection: string) {
    const existing = collectionColors.get(collection);
    if (existing) return existing;
    const color = COLORS[collectionColors.size % COLORS.length] ?? COLORS[0];
    collectionColors.set(collection, color);
    return color;
  }

  function addNode(id: string, fields: Record<string, unknown> = {}) {
    const collection = collectionOf(id);
    const existing = nodes.get(id);
    if (existing && Object.keys(existing.fields).length >= Object.keys(fields).length) return;
    nodes.set(id, { id, label: labelOf(id, fields), collection, color: colorFor(collection), fields });
  }

  function addLink(source: string, target: string, label: string) {
    const key = `${source}->${target}:${label}`;
    if (linkKeys.has(key)) return;
    linkKeys.add(key);
    links.push({ source, target, label });
  }

  for (const statement of result.allResults) {
    for (const row of statement.data) {
      const rowId = row.id;
      if (isDocumentId(rowId)) {
        const fields = { ...row };
        delete fields.id;
        addNode(rowId, fields);
      }
      for (const [field, value] of Object.entries(row)) {
        if (!Array.isArray(value)) continue;
        value.forEach((item, index) => {
          if (!item || typeof item !== "object" || Array.isArray(item)) return;
          const object = item as Record<string, unknown>;
          if (isDocumentId(object.from) && isDocumentId(object.to)) {
            addNode(object.from);
            addNode(object.to);
            addLink(object.from, object.to, field);
          } else if (isDocumentId(object.id)) {
            const fields = { ...object };
            delete fields.id;
            addNode(object.id, fields);
            if (isDocumentId(rowId)) addLink(rowId, object.id, field);
          } else if (isDocumentId(rowId) && (typeof object.name === "string" || typeof object.label === "string")) {
            const syntheticId = `${field}:${index}`;
            addNode(syntheticId, object);
            addLink(rowId, syntheticId, field);
          }
        });
      }
    }
  }
  return { nodes: [...nodes.values()], links };
}
