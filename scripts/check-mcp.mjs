import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { fileURLToPath } from "node:url";

const client = new Client({ name: "course-captain-doctor", version: "1" });
const transport = new StdioClientTransport({
  command: process.execPath,
  args: [fileURLToPath(new URL("./start-mcp.mjs", import.meta.url))],
  stderr: "inherit",
});
let timer;
try {
  await Promise.race([
    client.connect(transport),
    new Promise((_, reject) => {
      timer = setTimeout(
        () => reject(new Error("MCP startup timed out after 60 seconds")),
        60_000,
      );
    }),
  ]);
  clearTimeout(timer);
  const { tools } = await client.listTools();
  const overview = await client.callTool({
    name: "get_workspace_overview",
    arguments: {},
  });
  if (overview.isError)
    throw new Error(
      "MCP connected, but workspace overview failed. Check workspace access.",
    );
  console.log(
    `Course Captain MCP connected: ${tools.length} tools; workspace readable; no web server required.`,
  );
} finally {
  clearTimeout(timer);
  await client.close();
}
