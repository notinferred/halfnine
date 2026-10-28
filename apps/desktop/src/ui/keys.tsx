/** A shortcut drawn as keycaps, one per key, so “⌘ .” can't be read as “⌘” followed by a full stop. */
export function Keys({ keys }: { keys: string[] }) {
  return (
    <span className="keys">
      {keys.map((key) => (
        <kbd key={key}>{key}</kbd>
      ))}
    </span>
  );
}
