Bundled MCP server examples — all shipped with `enabled = false` so quack makes no
network calls and spawns no processes until you opt in.

## Quickest ways to enable or add a server

1. **Inside quack (TUI):** run `/mcp`, pick any bundled example (`context7`, `roost`, …)
   and press `Enter` to copy it to your user config, enable it, and connect immediately —
   or pick `+ Add remote MCP server (HTTP)` / `+ Add local MCP server (stdio)`.
2. **From the CLI:**
   ```sh
   quack mcp list
   quack mcp enable context7
   quack mcp add playwright -- npx -y @playwright/mcp@latest
   quack mcp add roost --url https://roost.internal:8484/mcp --header "Authorization=Bearer {env:ROOST_TOKEN}"
   ```
   Or run `quack mcp add` with no flags for the guided wizard.
3. **By hand:** run `quack config paths` to see your `mcp.d/` folder, copy one of these
   files into it, and delete `enabled = false` (or change it to `enabled = true`).

## Shipped examples

| File | Transport | Description |
|---|---|---|
| `context7.toml` | HTTP (or stdio) | Up-to-date library and framework docs via Context7 (works without a key; optional `CONTEXT7_API_KEY`) |
| `roost.toml` | HTTP (or stdio) | Self-hosted internal document search over Office, PDF, and text files ([ClackStack/roost](https://github.com/ClackStack/roost); uses `ROOST_TOKEN`) |
| `github.toml` | HTTP | GitHub repositories, issues, and pull requests (uses `GITHUB_TOKEN`) |
| `fetch.toml` | stdio (`uvx`) | Fetch web pages and convert HTML to Markdown (`mcp-server-fetch`) |
| `playwright.toml` | stdio (`npx`) | Browser automation and page inspection (`@playwright/mcp`) |
| `filesystem.toml` | stdio (`npx`) | Directory-scoped file tools (`@modelcontextprotocol/server-filesystem`, default `~/notes`) |
| `time.toml` | stdio (`uvx`) | Current time and timezone conversions (`mcp-server-time`) |
| `everything.toml` | stdio (`npx`) | Reference MCP test server (`@modelcontextprotocol/server-everything`) |
