export type DbValue =
  | null
  | boolean
  | string
  | number
  | DbValue[]
  | { [key: string]: DbValue }
  | { $type: "int64"; value: string }
  | { $type: "decimal"; value: string }
  | { $type: "datetime"; value: string }
  | { $type: "duration"; value: string }
  | { $type: "bytes"; value: string }
  | { $type: "reference"; value: string }
  | { $type: "range"; start: DbValue; end: DbValue };

export type DecodedDbScalar =
  | { kind: "int64"; value: string }
  | { kind: "decimal"; value: string }
  | { kind: "datetime"; value: string }
  | { kind: "duration"; value: string }
  | { kind: "bytes"; value: string }
  | { kind: "reference"; value: string };

export type DecodedDbValue =
  | null
  | boolean
  | string
  | number
  | DecodedDbValue[]
  | { [key: string]: DecodedDbValue }
  | DecodedDbScalar
  | { kind: "range"; start: DecodedDbValue; end: DecodedDbValue };

type TypedScalarKind = DecodedDbScalar["kind"];
const scalarTypes = new Set<string>([
  "int64",
  "decimal",
  "datetime",
  "duration",
  "bytes",
  "reference",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function decodeDbValue(value: unknown): DecodedDbValue {
  if (
    value === null ||
    typeof value === "boolean" ||
    typeof value === "string" ||
    typeof value === "number"
  ) {
    return value;
  }
  if (Array.isArray(value)) return value.map(decodeDbValue);
  if (!isRecord(value)) {
    throw new Error(`Unsupported DbValue payload: ${typeof value}`);
  }

  const type = value.$type;
  if (typeof type !== "string") {
    return Object.fromEntries(
      Object.entries(value).map(([key, entry]) => [key, decodeDbValue(entry)]),
    );
  }
  if (scalarTypes.has(type)) {
    if (typeof value.value !== "string") throw new Error(`Invalid ${type} DbValue`);
    return { kind: type as TypedScalarKind, value: value.value };
  }
  if (type === "range") {
    if (!("start" in value) || !("end" in value)) {
      throw new Error("Invalid range DbValue");
    }
    return {
      kind: "range",
      start: decodeDbValue(value.start),
      end: decodeDbValue(value.end),
    };
  }
  throw new Error(`Unknown DbValue type: ${type}`);
}
