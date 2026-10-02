# macsploit-mcp

An MCP server that lets AI agents run Luau in a live Roblox client through MacSploit, while the MacSploit app stays open and usable.

Unofficial; not affiliated with MacSploit or Roblox.

## How it works

MacSploit's injected library accepts only one client on port `5553`, and the MacSploit app holds that connection, so other tools can't use it. Instead, `macsploit-mcp` installs `MCPBridge.lua` into `~/Documents/Macsploit Automatic Execution/`. MacSploit runs it on every game join, and it long-polls the server on `127.0.0.1:8766` for jobs. Port `5553` is never touched.

## Tools

- **`execute(code | file, timeout_secs?)`**: runs inline Luau, or a local `.lua`/`.luau` file, with the full MacSploit ([sUNC](https://docs.sunc.io)) API. Returns `print`/`warn` output (also written to the Roblox console) and return values as JSON. Waits 30s by default (max 600). Results over ~3K tokens are trimmed, with the full result saved to a file.
- **`dump_scripts(filter?)`**: decompiles the game's client scripts into `.luau` files mirroring the instance tree, for the agent to read and search. `filter` limits it to paths containing the given text.

If the client was kicked or disconnected, results include a `disconnected` reason; the agent can rejoin with `TeleportService` through `execute`.

Files are written to `.macsploit/` in the working directory (with its own `.gitignore`), or to `$TMPDIR/macsploit-mcp/` when there is no project directory.

## Setup

1. Build:
   ```sh
   cargo build --release
   ```
2. Add `target/release/macsploit-mcp` to your MCP client. For Claude Code:
   ```sh
   claude mcp add macsploit -- /absolute/path/to/macsploit-mcp
   ```
   For Codex, in `~/.codex/config.toml`:
   ```toml
   [mcp_servers.macsploit]
   command = "/absolute/path/to/macsploit-mcp"
   ```
   For clients using an `mcpServers` JSON config (Pi's `~/.pi/agent/mcp.json`, Cursor, …):
   ```json
   { "mcpServers": { "macsploit": { "command": "/absolute/path/to/macsploit-mcp" } } }
   ```
3. Start your client, then open Roblox with MacSploit and join a game. The server installs the bridge on startup; if you were already in a game, rejoin once so autoexec runs it.

Long `execute` calls send a progress notification every 20s, which keeps clients like Pi waiting past their 60s default. Clients that don't reset their timeout on progress need a longer one, e.g. Codex `tool_timeout_sec = 620`.

Only one game session is supported at a time. Several MCP clients can run at once; they share the first server's connection.

## Credits

Decompilation uses [Opiumware](https://discord.gg/opiumware)'s decompiler, vendored in `vendor/` and embedded in the binary.

## License

MIT
