import { type ReactNode, useEffect, useRef, useState } from "react";

export type MenuItem = {
  label: string;
  onSelect: () => void;
  /** Marks the item that is chosen now, such as the connection shown. */
  checked?: boolean;
  /** The shortcut that does the same, shown at the item's end. */
  keys?: string;
  /** Draws a line above the item, to set it apart from those before. */
  apart?: boolean;
};

/** A button that opens a short list of actions, closed by choosing one, clicking elsewhere or pressing Escape. */
export function Menu(props: {
  label: string;
  icon: ReactNode;
  items: MenuItem[];
  disabled?: boolean;
  align?: "start" | "end";
  /** The button's own look in place of a quiet button's, as the sidebar's connection header has. */
  className?: string;
  title?: string;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const away = (event: MouseEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", away);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", away);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="menu" ref={root}>
      <button
        type="button"
        className={props.className ?? "btn btn-quiet menu-button"}
        title={props.title}
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={props.disabled}
        onClick={() => setOpen(!open)}
      >
        {props.icon}
        {props.label}
      </button>
      {open && (
        <div className={`menu-list menu-${props.align ?? "end"}`} role="menu">
          {props.items.map((item, i) => (
            <button
              key={`${i}-${item.label}`}
              type="button"
              role="menuitem"
              aria-current={item.checked || undefined}
              className={item.apart ? "menu-apart" : undefined}
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
            >
              {item.checked !== undefined && <span className="menu-check">{item.checked ? "✓" : ""}</span>}
              <span className="menu-label">{item.label}</span>
              {item.keys && <span className="menu-keys">{item.keys}</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
