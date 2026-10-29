import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { commands, type ProposalView } from "../../ipc/bindings.ts";
import type { ProposalTab as Tab, TableTab as TableTabState } from "../../state.ts";
import { fromChanges, noEdits } from "../table/edits.ts";
import { draftWith } from "../table/structure-draft.ts";
import { TableTab } from "../table/table-tab.tsx";
import { ApprovalBar } from "./approval-bar.tsx";

/** What a tab says for an agent's proposal: the table, and the connection when it isn't the one open. */
export function proposalTitle(proposal: ProposalView | undefined, connectionId: string) {
  if (!proposal) return "Agent's change";
  const what = proposal.table?.name ?? "SQL";
  return proposal.session.connection.id === connectionId ? what : `${what} · ${proposal.session.connection.name}`;
}

/** Whether a structure change replaces the table as a whole, so there is no draft of it to show, only its SQL. */
const wholeTable = (proposal: ProposalView) =>
  proposal.changes?.kind === "structure" &&
  proposal.changes.changes.some((c) => c.kind === "createTable" || c.kind === "drop" || c.kind === "truncate");

/** An agent's change, staged on the agent's own session the way the person's own edits are staged, until they answer. */
export function ProposalTab({ tab }: { tab: Tab }) {
  const agents = useQuery({ queryKey: ["agents"], queryFn: commands.agentState });
  const proposal = agents.data?.proposals.find((p) => p.id === tab.proposalId);
  if (!proposal) {
    return (
      <div className="workspace-empty">
        <p className="muted">This change is no longer waiting.</p>
      </div>
    );
  }
  if (proposal.kind === "statements" || !proposal.table || wholeTable(proposal)) return <ProposedSql proposal={proposal} />;
  return <ProposedTable tabId={tab.id} proposal={proposal} />;
}

function ProposedTable({ tabId, proposal }: { tabId: string; proposal: ProposalView }) {
  const table = proposal.table ?? { schema: "public", name: "" };
  const changes = proposal.changes;
  const structure = useQuery({
    queryKey: ["structure", proposal.session.sessionId, table.schema, table.name],
    queryFn: () => commands.structure(proposal.session.sessionId, table),
    enabled: changes?.kind === "structure",
  });
  const tab = useMemo<TableTabState>(
    () => ({
      kind: "table",
      id: tabId,
      table,
      filter: proposal.filter,
      edits: changes ? fromChanges(changes) : noEdits,
      draft: changes?.kind === "structure" && structure.data ? draftWith(structure.data, changes.changes) : null,
    }),
    [tabId, table, proposal.filter, changes, structure.data],
  );
  return <TableTab session={proposal.session} tab={tab} proposal={proposal} />;
}

function ProposedSql({ proposal }: { proposal: ProposalView }) {
  return (
    <div className="proposal-sql">
      <p className="proposal-where muted">
        {proposal.session.connection.name} · {proposal.session.server.database}
      </p>
      <ApprovalBar proposal={proposal} reviewOpen />
    </div>
  );
}
