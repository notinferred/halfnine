import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { type ColumnInfo, commands, type Rule, type RuleCheck, type SessionInfo, type Suggestion, type TableRef } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";

type Kind = RuleCheck["kind"];

const KINDS: { kind: Kind; name: string }[] = [
  { kind: "alwaysFilled", name: "is always filled" },
  { kind: "oneOf", name: "is one of a set of values" },
  { kind: "between", name: "stays between bounds" },
  { kind: "hasMatch", name: "has a matching row in another table" },
  { kind: "onlyGrows", name: "Rows are only added" },
  { kind: "custom", name: "Your own query" },
];

const SLOW =
  "Checking it takes over a second on this table, so changes to the table may wait for it, or be refused for time. A real constraint or an index would be faster.";

const sameTable = (a: TableRef, b: TableRef) => a.schema === b.schema && a.name === b.name;

/** A rule's source in words: the person's own, or what in the data suggested it. */
const sourceInWords = (rule: Rule) => (rule.source.kind === "suggested" ? `Suggested: ${rule.source.evidence}` : "Yours");

/**
 * What correct data is in this table, as rules the person sets: an agent's change that breaks one never commits, and
 * the person's own commit warns. Halfnine suggests rules from patterns in the data.
 */
export function RulesSection(props: { session: SessionInfo; table: TableRef; columns: ColumnInfo[] }) {
  const id = props.session.sessionId;
  const queries = useQueryClient();
  const rules = useQuery({ queryKey: ["rules", id], queryFn: () => commands.listRules(id) });
  const mine = (rules.data ?? []).filter((r) => sameTable(r.table, props.table));
  const [adding, setAdding] = useState(false);
  const [suggesting, setSuggesting] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);

  const added = (slow: boolean) => {
    void queries.invalidateQueries({ queryKey: ["rules", id] });
    setNote(slow ? { text: SLOW } : null);
  };
  const remove = async (rule: Rule) => {
    try {
      await commands.deleteRule(id, rule.id);
      void queries.invalidateQueries({ queryKey: ["rules", id] });
    } catch (e) {
      setNote({ text: describe(e), error: true });
    }
  };

  return (
    <section>
      <h2>Rules</h2>
      {mine.length === 0 && <p className="muted rules-empty">What correct data is here. An agent's change that breaks a rule never commits.</p>}
      {mine.length > 0 && (
        <div className="structure-list">
          {mine.map((rule) => (
            <div key={rule.id} className="structure-item rules-item">
              <span className="rules-words">{rule.words}</span>
              <span className="muted rules-source">{sourceInWords(rule)}</span>
              <button type="button" className="btn btn-quiet" onClick={() => void remove(rule)}>
                Delete
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="rules-actions">
        <button type="button" className="btn btn-quiet" onClick={() => setAdding(true)}>
          Add rule…
        </button>
        <button type="button" className="btn btn-quiet" onClick={() => setSuggesting(true)}>
          Suggest from the data…
        </button>
      </div>
      {note && <p className={note.error ? "status-error rules-note" : "muted rules-note"}>{note.text}</p>}
      {adding && (
        <AddRuleSheet
          session={props.session}
          table={props.table}
          columns={props.columns}
          onClose={() => setAdding(false)}
          onAdded={(slow) => {
            setAdding(false);
            added(slow);
          }}
        />
      )}
      {suggesting && <SuggestSheet session={props.session} table={props.table} onClose={() => setSuggesting(false)} onAdded={added} />}
    </section>
  );
}

function AddRuleSheet(props: {
  session: SessionInfo;
  table: TableRef;
  columns: ColumnInfo[];
  onClose: () => void;
  onAdded: (slow: boolean) => void;
}) {
  const id = props.session.sessionId;
  const first = props.columns[0]?.name ?? "";
  const [kind, setKind] = useState<Kind>("alwaysFilled");
  const [column, setColumn] = useState(first);
  const [values, setValues] = useState("");
  const [atLeast, setAtLeast] = useState("");
  const [atMost, setAtMost] = useState("");
  const [target, setTarget] = useState("");
  const [targetColumn, setTargetColumn] = useState("id");
  const [sql, setSql] = useState("");
  const [words, setWords] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const schema = useQuery({ queryKey: ["schema", id], queryFn: () => commands.schema(id) });
  const tables = (schema.data ?? []).flatMap((node) =>
    node.relations.filter((r) => r.kind === "table" || r.kind === "partitionedTable").map((r) => `${node.name}.${r.name}`),
  );
  const kindSelect = useRef<HTMLSelectElement>(null);
  useEffect(() => kindSelect.current?.focus(), []);

  const check = (): RuleCheck => {
    if (kind === "alwaysFilled") return { kind, column };
    if (kind === "oneOf")
      return {
        kind,
        column,
        values: values
          .split(",")
          .map((v) => v.trim())
          .filter(Boolean),
      };
    if (kind === "between") return { kind, column, atLeast: atLeast.trim() || null, atMost: atMost.trim() || null };
    if (kind === "hasMatch") {
      const [schemaName = "", name = ""] = target.split(/\.(.*)/s);
      return { kind, column, target: { schema: schemaName, name }, targetColumn: targetColumn.trim() };
    }
    if (kind === "onlyGrows") return { kind };
    return { kind, sql, reads: [] };
  };

  const add = async () => {
    setBusy(true);
    setError(null);
    try {
      const done = await commands.addRule(id, props.table, check(), kind === "custom" ? words : null, null);
      props.onAdded(done.slow);
    } catch (e) {
      setError(describe(e));
      setBusy(false);
    }
  };

  const columnPicker = (
    <select aria-label="Column" value={column} onChange={(e) => setColumn(e.target.value)}>
      {props.columns.map((c) => (
        <option key={c.name} value={c.name}>
          {c.name}
        </option>
      ))}
    </select>
  );
  const needsColumn = kind !== "onlyGrows" && kind !== "custom";

  return (
    <div className="sheet-layer">
      <button type="button" className="sheet-scrim" aria-label="Cancel" tabIndex={-1} onClick={props.onClose} />
      <form
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="add-rule-title"
        onSubmit={(e) => {
          e.preventDefault();
          if (!busy) void add();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onClose();
        }}
      >
        <h2 id="add-rule-title">A rule for {props.table.name}</h2>
        <p className="muted">Halfnine tries it on the data first. Rows that break it already are named, and it isn't added.</p>
        <div className="rules-sentence">
          {needsColumn && columnPicker}
          <select ref={kindSelect} aria-label="Rule" value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
            {KINDS.map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.name}
              </option>
            ))}
          </select>
        </div>
        {kind === "oneOf" && (
          <>
            <label className="sheet-label" htmlFor="rule-values">
              Values, separated by commas
            </label>
            <input id="rule-values" className="input" value={values} onChange={(e) => setValues(e.target.value)} placeholder="draft, sent, paid" />
          </>
        )}
        {kind === "between" && (
          <div className="rules-sentence">
            <label className="sheet-label" htmlFor="rule-at-least">
              At least
            </label>
            <input id="rule-at-least" className="input" value={atLeast} onChange={(e) => setAtLeast(e.target.value)} />
            <label className="sheet-label" htmlFor="rule-at-most">
              At most
            </label>
            <input id="rule-at-most" className="input" value={atMost} onChange={(e) => setAtMost(e.target.value)} />
          </div>
        )}
        {kind === "hasMatch" && (
          <div className="rules-sentence">
            <select aria-label="Table" value={target} onChange={(e) => setTarget(e.target.value)}>
              <option value="">Choose a table</option>
              {tables.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
            <label className="sheet-label" htmlFor="rule-target-column">
              with that
            </label>
            <input id="rule-target-column" className="input" value={targetColumn} onChange={(e) => setTargetColumn(e.target.value)} />
          </div>
        )}
        {kind === "onlyGrows" && <p className="muted">Rows are added to {props.table.name}, never changed or removed.</p>}
        {kind === "custom" && (
          <>
            <label className="sheet-label" htmlFor="rule-sql">
              A query that finds the rows breaking the rule
            </label>
            <textarea
              id="rule-sql"
              className="input mono rules-sql"
              spellCheck={false}
              value={sql}
              onChange={(e) => setSql(e.target.value)}
              placeholder="SELECT id FROM invoices WHERE total <> (SELECT sum(amount) FROM lines WHERE lines.invoice_id = invoices.id)"
            />
            <label className="sheet-label" htmlFor="rule-words">
              The rule in words, as agents will read it
            </label>
            <input id="rule-words" className="input" value={words} onChange={(e) => setWords(e.target.value)} />
          </>
        )}
        {error && (
          <p className="status-error" role="alert">
            {error}
          </p>
        )}
        <div className="sheet-actions">
          <button type="button" className="btn" onClick={props.onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={busy}>
            {busy ? "Trying it…" : "Add rule"}
          </button>
        </div>
      </form>
    </div>
  );
}

function SuggestSheet(props: { session: SessionInfo; table: TableRef; onClose: () => void; onAdded: (slow: boolean) => void }) {
  const id = props.session.sessionId;
  const found = useQuery({
    queryKey: ["suggest-rules", id, props.table.schema, props.table.name],
    queryFn: () => commands.suggestRules(id, props.table),
    staleTime: 0,
  });
  const [done, setDone] = useState<Set<string>>(new Set());
  const [errors, setErrors] = useState<Record<string, string>>({});
  const left = (found.data ?? []).filter((s) => !done.has(s.words));

  const accept = async (suggestion: Suggestion) => {
    try {
      const added = await commands.addRule(id, props.table, suggestion.check, null, suggestion.evidence);
      setDone(new Set(done).add(suggestion.words));
      props.onAdded(added.slow);
    } catch (e) {
      setErrors({ ...errors, [suggestion.words]: describe(e) });
    }
  };

  return (
    <div className="sheet-layer">
      <button type="button" className="sheet-scrim" aria-label="Close" tabIndex={-1} onClick={props.onClose} />
      <div
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="suggest-title"
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onClose();
        }}
      >
        <h2 id="suggest-title">Rules the data suggests for {props.table.name}</h2>
        {found.isPending && <p className="muted">Looking at the data…</p>}
        {found.isError && <p className="status-error">{describe(found.error)}</p>}
        {found.data && left.length === 0 && <p className="muted">Nothing more stands out in the data.</p>}
        <div className="structure-list">
          {left.map((s) => (
            <div key={s.words} className="structure-item rules-item">
              <span className="rules-words">{s.words}</span>
              <span className={errors[s.words] ? "status-error rules-source" : "muted rules-source"}>{errors[s.words] ?? s.evidence}</span>
              <button type="button" className="btn btn-quiet" onClick={() => setDone(new Set(done).add(s.words))}>
                Dismiss
              </button>
              <button type="button" className="btn" onClick={() => void accept(s)}>
                Accept
              </button>
            </div>
          ))}
        </div>
        <div className="sheet-actions">
          <button type="button" className="btn" onClick={props.onClose}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
