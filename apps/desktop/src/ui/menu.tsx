import { type ReactNode, useEffect, useRef, useState } from "react";

export type MenuItem = { label: string; onSelect: () => void };

/** A button that opens a short list of actions, closed by choosing one, clicking elsewhere or pressing Escape. */
export function Menu(props: { label: string; icon: ReactNode; items: MenuItem[]; disabled?: boolean; align?: "start" | "end" }) {
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
        className="btn btn-quiet menu-button"
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
          {props.items.map((item) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
