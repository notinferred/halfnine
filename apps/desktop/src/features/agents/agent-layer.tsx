import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { commands, events } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { openOn, useWorkspace } from "../../state.ts";
import { showNote } from "../../ui/note.tsx";
import { PasswordSheet } from "../connections/password-sheet.tsx";
import { DeletionSheet } from "./deletion-sheet.tsx";

/** Lets go of the Docker databases open in the window once they're gone, as when an agent deleted one it made. */
async function leaveIfDeleted() {
  const { session, parked, forgetConnection } = useWorkspace.getState();
  const open = [session, ...parked.map((p) => p.session)].filter((s) => s?.connection.source?.kind === "docker");
  if (open.length === 0) return;
  const state = await commands.dockerState().catch(() => null);
  if (!state?.on || state.problem) return;
  for (const gone of open) {
    if (!gone || state.databases.some((d) => d.connection.id === gone.connection.id)) continue;
    forgetConnection(gone.connection.id);
    showNote({ text: `${gone.connection.name} was deleted.` });
  }
}

/**
 * Keeps the window in step with what agents wait on: a tab for each proposal, opening its connection first from the
 * connections screen, the password box when an agent needs one, and the question when an agent asks to delete a
 * database it didn't make. The app holds the state and says when it changes; this only follows it.
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
    const docker = events.dockerChanged.listen(() => {
      void queries.invalidateQueries({ queryKey: ["docker"] });
      void queries.invalidateQueries({ queryKey: ["connections"] });
      void leaveIfDeleted();
    });
    return () => {
      void changed.then((stop) => stop());
      void logged.then((stop) => stop());
      void docker.then((stop) => stop());
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
      // A proposal on a connection already open in the window shows that one.
      const already = openOn(useWorkspace.getState(), first.session.connection.id);
      if (already) {
        useWorkspace.getState().show(already.sessionId);
        return;
      }
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

  const deletion = agents.data?.deletions[0];
  if (deletion) return <DeletionSheet key={deletion.id} request={deletion} />;
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
