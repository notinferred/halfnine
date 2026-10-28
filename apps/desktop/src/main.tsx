import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app.tsx";
import { isTransient } from "./ipc/errors.ts";
import { isMac } from "./ui/shortcuts.ts";
import "./ui/base.css";

document.documentElement.dataset.platform = isMac ? "mac" : "windows";

// The database changes only when someone refreshes (⌘R), so nothing is refetched behind their back: not on focus, not on a timer.
const queries = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: Number.POSITIVE_INFINITY,
      refetchOnWindowFocus: false,
      retry: (failures, error) => failures < 1 && isTransient(error),
    },
  },
});

const root = document.getElementById("root");
if (!root) throw new Error("index.html has no #root");

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queries}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
);
