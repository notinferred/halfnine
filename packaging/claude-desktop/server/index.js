// Starts the Halfnine installed on this computer as Claude Desktop's MCP server. Halfnine does the rest: it opens the
// app if it is closed, applies each connection's access for agents, and asks in its window before a change.
const { spawn } = require("node:child_process");
const { existsSync } = require("node:fs");
const { homedir } = require("node:os");
const { join } = require("node:path");

const local = process.env.LOCALAPPDATA;
const installed =
  process.platform === "win32"
    ? [local && join(local, "Halfnine", "halfnine.exe"), local && join(local, "Programs", "Halfnine", "halfnine.exe")]
    : ["/Applications/Halfnine.app/Contents/MacOS/halfnine", join(homedir(), "Applications", "Halfnine.app", "Contents", "MacOS", "halfnine")];
const app = installed.find((path) => path && existsSync(path));

if (!app) {
  process.stderr.write("Halfnine isn't installed. Get it from https://github.com/notinferred/halfnine/releases, then restart Claude Desktop.\n");
  process.exit(1);
}

const server = spawn(app, ["mcp"], { stdio: "inherit" });
server.on("exit", (code) => process.exit(code ?? 1));
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => server.kill(signal));
