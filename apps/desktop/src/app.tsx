import { AgentLayer } from "./features/agents/agent-layer.tsx";
import { ConnectionsScreen } from "./features/connections/connections-screen.tsx";
import { Workspace } from "./features/workspace/workspace.tsx";
import { useWorkspace } from "./state.ts";
import { NoteLayer } from "./ui/note.tsx";

export function App() {
  const session = useWorkspace((s) => s.session);
  return (
    <>
      {session ? <Workspace session={session} /> : <ConnectionsScreen />}
      <AgentLayer />
      <NoteLayer />
    </>
  );
}
