const since = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

/** How long ago `ms` (milliseconds since 1970) was, in words: "3 minutes ago". */
export function ago(ms: number | null): string {
  const seconds = Math.round(((ms ?? 0) - Date.now()) / 1000);
  for (const [unit, size] of [
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
  ] as const) {
    if (Math.abs(seconds) >= size) return since.format(Math.round(seconds / size), unit);
  }
  return since.format(seconds, "second");
}
