import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Command } from "cmdk";
import { Check, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { commands, type ForeignKey, type SessionInfo } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import type { Cell } from "./edits.ts";
import "../palette/palette.css";

/** Sets a foreign-key cell by picking the row it should point to, found by its key or its name. */
export function KeyPicker(props: {
  session: SessionInfo;
  foreignKey: ForeignKey;
  current: Cell | undefined;
  nullable: boolean;
  onPick: (value: Cell) => void;
  onClose: () => void;
}) {
  const id = props.session.sessionId;
  const { target, targetColumns } = props.foreignKey;
  const column = targetColumns[0] ?? "";
  const [search, setSearch] = useState("");
  const [asked, setAsked] = useState("");
  useEffect(() => {
    const wait = setTimeout(() => setAsked(search), 150);
    return () => clearTimeout(wait);
  }, [search]);
  const choices = useQuery({
    queryKey: ["choices", id, target.schema, target.name, column, asked],
    queryFn: () => commands.choices(id, target, column, asked),
    placeholderData: keepPreviousData,
  });
  const pick = (value: Cell) => {
    props.onPick(value);
    props.onClose();
  };

  return (
    <div className="palette-layer">
      <button type="button" className="palette-scrim" aria-label="Close" tabIndex={-1} onClick={props.onClose} />
      <Command
        className="palette"
        label={`Choose a ${target.name} row`}
        shouldFilter={false}
        loop
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onClose();
        }}
      >
        <div className="palette-input">
          <Search size={16} strokeWidth={1.5} aria-hidden="true" />
          <Command.Input autoFocus value={search} onValueChange={setSearch} placeholder={`Find a ${target.name} row`} />
          <kbd>esc</kbd>
        </div>
        <Command.List className="palette-list">
          <Command.Empty className="palette-empty">
            {choices.isError ? describe(choices.error) : choices.isPending ? "Loading…" : "Nothing matches."}
          </Command.Empty>
          {props.nullable && !search && (
            <Command.Item value="null" onSelect={() => pick(null)}>
              <span className="palette-name grid-null">NULL</span>
              <span className="palette-detail">Points to no row</span>
              {props.current === null && <Check className="palette-keys" size={14} strokeWidth={1.5} aria-label="Current" />}
            </Command.Item>
          )}
          {choices.data?.map((choice) => (
            <Command.Item key={choice.value} value={choice.value} onSelect={() => pick(choice.value)}>
              <span className="palette-name mono">{choice.value}</span>
              {choice.label !== null && <span className="palette-detail">{choice.label}</span>}
              {props.current === choice.value && <Check className="palette-keys" size={14} strokeWidth={1.5} aria-label="Current" />}
            </Command.Item>
          ))}
        </Command.List>
        <div className="palette-hints">
          <span>
            {target.name}.{column}
          </span>
          <span>The first 50 rows. Type to find others.</span>
        </div>
      </Command>
    </div>
  );
}
