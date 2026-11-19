// Starts the Okayrow installed on this computer as Claude Desktop's MCP server. Okayrow does the rest: it opens the
// app if it is closed, applies each connection's access for agents, and asks in its window before a change.
const { spawn } = require("node:child_process");
const { existsSync } = require("node:fs");
const { homedir } = require("node:os");
const { join } = require("node:path");

const local = process.env.LOCALAPPDATA;
const installed =
  process.platform === "win32"
    ? [local && join(local, "Okayrow", "okayrow.exe"), local && join(local, "Programs", "Okayrow", "okayrow.exe")]
    : ["/Applications/Okayrow.app/Contents/MacOS/okayrow", join(homedir(), "Applications", "Okayrow.app", "Contents", "MacOS", "okayrow")];
const app = installed.find((path) => path && existsSync(path));

if (!app) {
  process.stderr.write("Okayrow isn't installed. Get it from https://github.com/notinferred/okayrow/releases, then restart Claude Desktop.\n");
  process.exit(1);
}

const server = spawn(app, ["mcp"], { stdio: "inherit" });
server.on("exit", (code) => process.exit(code ?? 1));
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => server.kill(signal));
