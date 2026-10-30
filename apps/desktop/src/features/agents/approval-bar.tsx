import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { commands, type ProposalView } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";

/**
 * The window comes forward when an agent asks, possibly while the person is still typing elsewhere, so no choice
 * works until the window has had focus for a second, and losing focus disarms them again.
 */
export function useArmed(delay = 1000) {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    let timer: number | undefined;
    const arm = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => setArmed(true), delay);
    };
    const disarm = () => {
      window.clearTimeout(timer);
      setArmed(false);
    };
    if (document.hasFocus()) arm();
    window.addEventListener("focus", arm);
    window.addEventListener("blur", disarm);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", arm);
      window.removeEventListener("blur", disarm);
    };
  }, [delay]);
  return armed;
}

export const isTyping = (target: EventTarget | null) =>
  target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable);

/**
 * An agent's change, asked the way Claude Code asks: 1. Yes, 2. Yes, don't ask again here, 3. No, and tell it why.
 * There is no default: Enter does nothing until the person picks one.
 */
export function ApprovalBar(props: { proposal: ProposalView; reviewOpen?: boolean }) {
  const { proposal } = props;
  const queries = useQueryClient();
  const armed = useArmed();
  const [reviewing, setReviewing] = useState(props.reviewOpen ?? false);
  const [telling, setTelling] = useState(false);
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const reasonField = useRef<HTMLInputElement>(null);
  const state = proposal.state;
  const committing = state.kind === "committing";
  const failed = state.kind === "failed" ? describe(state.error) : null;

  const approve = async (remember: boolean) => {
    setError(null);
    try {
      await commands.approveProposal(proposal.id, remember);
      // What the person has open of that table is now behind, so it loads again.
      void queries.invalidateQueries({ predicate: (q) => ["page", "structure", "schema"].includes(String(q.queryKey[0])) });
    } catch (e) {
      setError(describe(e));
    }
  };
  const decline = () => void commands.declineProposal(proposal.id, reason);

  const choose = (choice: 1 | 2 | 3) => {
    if (!armed || committing || failed) return;
    if (choice === 3) {
      setTelling(true);
      return;
    }
    void approve(choice === 2);
  };

  useEffect(() => {
    if (telling) reasonField.current?.focus();
  }, [telling]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (telling || isTyping(event.target) || event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === "1" || event.key === "2" || event.key === "3") {
        event.preventDefault();
        choose(Number(event.key) as 1 | 2 | 3);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <div className="changes approval">
      {reviewing && (
        <div className="review">
          <p className="muted">
            {proposal.kind === "statements"
              ? "These statements run in order in one transaction. If one fails, none of them is applied."
              : "These statements run together in one transaction. If a row has changed since the agent proposed this, nothing is written."}
          </p>
          <pre className="review-sql mono">{["BEGIN;", ...proposal.shown, "COMMIT;"].join("\n")}</pre>
        </div>
      )}
      {(error ?? failed) && (
        <p className="changes-note status-error" role="alert">
          {error ?? failed}
        </p>
      )}
      <div className="changes-bar">
        <span className="changes-dot" aria-hidden="true" />
        <span className="changes-summary">
          {proposal.agent} wants to {proposal.summary}
        </span>
        {!props.reviewOpen && (
          <button type="button" className="btn btn-quiet changes-review" aria-expanded={reviewing} onClick={() => setReviewing(!reviewing)}>
            Review SQL
          </button>
        )}
        {failed ? (
          <button type="button" className="btn" onClick={() => void commands.dismissProposal(proposal.id)}>
            Close
          </button>
        ) : committing ? (
          <>
            <span className="muted">Committing…</span>
            <button type="button" className="btn" onClick={() => void commands.cancelRun(proposal.session.sessionId)}>
              Stop
            </button>
          </>
        ) : telling ? (
          <form
            className="approval-reason"
            onSubmit={(e) => {
              e.preventDefault();
              decline();
            }}
          >
            <label htmlFor={`reason-${proposal.id}`} className="visually-hidden">
              What {proposal.agent} should do instead
            </label>
            <input
              id={`reason-${proposal.id}`}
              ref={reasonField}
              className="input"
              placeholder={`Tell ${proposal.agent} what to do instead`}
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") setTelling(false);
              }}
            />
            <button type="submit" className="btn btn-primary">
              Send
            </button>
          </form>
        ) : (
          <fieldset className="approval-choices" disabled={!armed} aria-label={`Answer ${proposal.agent}`}>
            <button type="button" className="btn btn-primary" onClick={() => choose(1)}>
              <kbd>1</kbd> Yes
            </button>
            <button type="button" className="btn" onClick={() => choose(2)}>
              <kbd>2</kbd> Yes, don't ask again here
            </button>
            <button type="button" className="btn" onClick={() => choose(3)}>
              <kbd>3</kbd> No, and tell it why
            </button>
          </fieldset>
        )}
      </div>
    </div>
  );
}
