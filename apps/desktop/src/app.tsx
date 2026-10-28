import { ConnectionsScreen } from "./features/connections/connections-screen.tsx";
import { Workspace } from "./features/workspace/workspace.tsx";
import { useWorkspace } from "./state.ts";

export function App() {
  const session = useWorkspace((s) => s.session);
  return session ? <Workspace session={session} /> : <ConnectionsScreen />;
}
