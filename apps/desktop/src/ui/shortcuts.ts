import { useEffect, useRef } from "react";
import { tinykeys } from "tinykeys";

/** Binds shortcuts for as long as the component is mounted. `$mod` is ⌘ on macOS and Ctrl on Windows. */
export function useShortcuts(bindings: Record<string, () => void>) {
  const latest = useRef(bindings);
  latest.current = bindings;
  const keys = Object.keys(bindings).join("|");

  useEffect(() => {
    const handlers = Object.fromEntries(
      keys.split("|").map((key) => [
        key,
        (event: KeyboardEvent) => {
          event.preventDefault();
          latest.current[key]?.();
        },
      ]),
    );
    // Every shortcut uses a modifier, so it can't clash with typing and works inside text fields too.
    return tinykeys(window, handlers, { ignore: () => false });
  }, [keys]);
}

export const isMac = navigator.userAgent.includes("Mac");
export const modKey = isMac ? "⌘" : "Ctrl+";
