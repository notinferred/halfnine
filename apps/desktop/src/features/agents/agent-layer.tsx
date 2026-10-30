import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { commands, events } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { PasswordSheet } from "../connections/password-sheet.tsx";

/**
 * Keeps the window in step with what agents wait on: a tab for each proposal, opening its connection first from the
 * connections screen, and the password box when an agent needs one. The app holds the state and says when it
 * changes; this only follows it.
 */
export function AgentLayer() {
  const queries = useQueryClient();
  const agents = useQuery({ queryKey: ["agents"], queryFn: commands.agentState });
  const session = useWorkspace((s) => s.session);
  const tabs = useWorkspace((s) => s.tabs);
  const { enter, openProposal, settleProposal, closeProposal } = useWorkspace.getState();
  const opening = useRef(false);
  const [answering, setAnswering] = useState(false);
  const [answerError, setAnswerError] = useState<string | null>(null);

  useEffect(() => {
    const changed = events.agentsChanged.listen(() => {
      void queries.invalidateQueries({ queryKey: ["agents"] });
      void queries.invalidateQueries({ queryKey: ["settings"] });
    });
    const logged = events.agentActivityAdded.listen(() => void queries.invalidateQueries({ queryKey: ["agentActivity"] }));
    return () => {
      void changed.then((stop) => stop());
      void logged.then((stop) => stop());
    };
  }, [queries]);

  const proposals = agents.data?.proposals;
  useEffect(() => {
    if (!proposals) return;
    const waiting = new Set(proposals.map((p) => p.id));
    for (const tab of tabs) if (tab.kind === "proposal" && !waiting.has(tab.proposalId)) closeProposal(tab.proposalId);
    if (proposals.length === 0) return;
    if (!session) {
      const first = proposals[0];
      if (!first || opening.current) return;
      opening.current = true;
      commands
        .openSession(first.session.connection.id, first.session.server.database, null)
        .then(enter)
        .catch(() => undefined)
        .finally(() => {
          opening.current = false;
        });
      return;
    }
    for (const proposal of proposals) {
      const tab = tabs.find((t) => t.kind === "proposal" && t.proposalId === proposal.id);
      if (!tab) openProposal(proposal.id);
      else if (tab.kind === "proposal" && !tab.settled && proposal.state.kind === "failed") settleProposal(proposal.id);
    }
  }, [proposals, session, tabs, enter, openProposal, settleProposal, closeProposal]);

  const asked = agents.data?.passwords[0];
  if (!asked) return null;
  return (
    <PasswordSheet
      key={asked.id}
      connection={asked.connection}
      asker={asked.agent}
      busy={answering}
      error={answerError}
      onConnect={(typed) => {
        setAnswering(true);
        setAnswerError(null);
        commands
          .answerPasswordRequest(asked.id, typed)
          .catch((error) => setAnswerError(describe(error)))
          .finally(() => setAnswering(false));
      }}
      onClose={() => void commands.cancelPasswordRequest(asked.id)}
    />
  );
}
