import type { Error as AppError } from "./bindings.ts";

const isAppError = (value: unknown): value is AppError =>
  typeof value === "object" && value !== null && "kind" in value && "message" in value && typeof value.message === "string";

/** A command's error in words, with Postgres's SQLSTATE beside it when there is one. */
export function describe(error: unknown): string {
  if (isAppError(error)) return error.code ? `${error.message} (${error.code})` : error.message;
  if (error instanceof Error) return error.message;
  return String(error);
}

/** The connection asks for its password each time, and it hasn't been typed yet this launch. */
export function needsPassword(error: unknown): boolean {
  return isAppError(error) && error.kind === "passwordNeeded";
}

/** Halfnine couldn't back up what a drop or truncate would lose, so nothing changed; the message says why. */
export function notBackedUp(error: unknown): boolean {
  return isAppError(error) && error.kind === "noBackup";
}

/** Network trouble and timeouts may pass; everything else needs someone to change something first. */
export function isTransient(error: unknown): boolean {
  return isAppError(error) && (error.kind === "network" || error.kind === "timeout");
}
