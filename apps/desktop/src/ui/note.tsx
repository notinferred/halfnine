import { X } from "lucide-react";
import { useState } from "react";
import { create } from "zustand";

export type Note = {
  text: string;
  /** A button beside the text, such as Undo. */
  action?: { label: string; run: () => Promise<void> };
  error?: boolean;
};

type Notes = { note: Note | null; shown: number; show: (note: Note) => void; dismiss: () => void };

const useNotes = create<Notes>((set) => ({
  note: null,
  shown: 0,
  show: (note) => set((s) => ({ note, shown: s.shown + 1 })),
  dismiss: () => set({ note: null }),
}));

/** Shows a note at the foot of the window, in place of any before it. */
export const showNote = (note: Note) => useNotes.getState().show(note);

/** The one note at the foot of the window, on either screen. It stays until another replaces it or it is closed. */
export function NoteLayer() {
  const { note, shown, dismiss } = useNotes();
  if (!note) return null;
  return <NoteView key={shown} note={note} onDismiss={dismiss} />;
}

function NoteView(props: { note: Note; onDismiss: () => void }) {
  const { note } = props;
  const [busy, setBusy] = useState(false);
  return (
    <div className={note.error ? "note note-error" : "note"} role={note.error ? "alert" : "status"}>
      <span className="note-text">{note.text}</span>
      {note.action && (
        <button
          type="button"
          className="btn note-action"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void note.action?.run().finally(() => setBusy(false));
          }}
        >
          {note.action.label}
        </button>
      )}
      <button type="button" className="note-close" aria-label="Close" onClick={props.onDismiss}>
        <X size={13} strokeWidth={1.75} aria-hidden="true" />
      </button>
    </div>
  );
}
