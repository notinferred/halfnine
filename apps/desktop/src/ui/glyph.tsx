// The brand mark: a 3 × 3 table whose top row is the header and whose centre cell is selected. assets/brand/app-icon.svg is the same drawing on its tile.
const cells = [0, 1, 2].flatMap((row) => [0, 1, 2].map((col) => ({ row, col })));

export function Glyph({ size }: { size: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 568 568" aria-hidden="true">
      {cells.map(({ row, col }) => (
        <rect
          key={`${row}-${col}`}
          x={col * 200}
          y={row * 200}
          width={168}
          height={168}
          rx={28}
          fill={row === 1 && col === 1 ? "var(--accent)" : "currentColor"}
          opacity={row === 1 && col === 1 ? 1 : row === 0 ? 0.32 : 0.14}
        />
      ))}
    </svg>
  );
}
