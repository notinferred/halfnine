import { useQuery } from "@tanstack/react-query";
import { AgentLayer } from "./features/agents/agent-layer.tsx";
import { ConnectionsScreen } from "./features/connections/connections-screen.tsx";
import { FirstRun } from "./features/settings/first-run.tsx";
import { Workspace } from "./features/workspace/workspace.tsx";
import { commands } from "./ipc/bindings.ts";
import { useWorkspace } from "./state.ts";
import { NoteLayer } from "./ui/note.tsx";

export function App() {
  const session = useWorkspace((s) => s.session);
  const settings = useQuery({ queryKey: ["settings"], queryFn: commands.getSettings });
  if (settings.data && !settings.data.setupSeen) return <FirstRun />;
  return (
    <>
      {session ? <Workspace session={session} /> : <ConnectionsScreen />}
      <AgentLayer />
      <NoteLayer />
    </>
  );
}
