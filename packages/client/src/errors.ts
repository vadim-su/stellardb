import type { StructuredErrorBody } from "./types.js";

export class StellarDbError extends Error {
  readonly status: number;
  readonly code?: string;
  readonly severity?: string;
  readonly detail?: string;
  readonly hint?: string;
  readonly statementIndex?: number;

  constructor(status: number, error: StructuredErrorBody["error"] | string) {
    const payload = typeof error === "string" ? { message: error } : error;
    super(payload.message);
    this.name = "StellarDbError";
    this.status = status;
    this.code = payload.code;
    this.severity = payload.severity;
    this.detail = payload.detail;
    this.hint = payload.hint;
    this.statementIndex = payload.statement_index;
  }
}

export async function errorFromResponse(
  response: Response,
): Promise<StellarDbError> {
  const body: unknown = await response.json().catch(() => null);
  if (
    body &&
    typeof body === "object" &&
    "error" in body &&
    body.error &&
    typeof body.error === "object" &&
    "message" in body.error &&
    typeof body.error.message === "string"
  ) {
    return new StellarDbError(
      response.status,
      body.error as StructuredErrorBody["error"],
    );
  }
  return new StellarDbError(response.status, `HTTP error: ${response.status}`);
}
