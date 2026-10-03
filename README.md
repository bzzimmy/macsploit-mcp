# macsploit-mcp

An MCP server that lets an AI agent run Luau in a live Roblox client through MacSploit. The MacSploit app stays open, and you can keep using it.

Unofficial. Not affiliated with MacSploit or Roblox.

## See it running

<p align="center">
  <img src="assets/demo.avif" width="900" alt="An agent building a Hide or Oof auto-kill cheat in a live Roblox client">
</p>
<p align="center"><em>Live Roblox on the left, the agent on the right, 6× speed. From one prompt:</em></p>

> For the game that I'm currently in "Hide or Oof" I would like you to make a script that when I'm a seeker and in the round will teleport to hiders one by one and kill all of them using the knife. it should have a simple UI with a toggle.

## How it works

MacSploit's injected library accepts only one client on port `5553`. The MacSploit app holds that connection, so other tools can't use it.

`macsploit-mcp` doesn't try. It installs `MCPBridge.lua` into `~/Documents/Macsploit Automatic Execution/`, MacSploit runs it on every game join, and the bridge long-polls the server on `127.0.0.1:8766` for jobs. Port `5553` is never touched.

## Tools

- `execute(code | file, timeout_secs?)` runs inline Luau or a local `.lua`/`.luau` file with the full MacSploit ([sUNC](https://docs.sunc.io)) API. It returns `print`/`warn` output (also shown in the Roblox console) and return values as JSON. It waits 30s by default (max 600). Results over ~3K tokens are trimmed and saved to a file.
- `dump_scripts(filter?)` decompiles the game's client scripts into `.luau` files mirroring the instance tree, so the agent can read and search them. Each dump replaces the previous one unless `filter` is set, in which case it covers only paths containing the given text.

If the client gets kicked or disconnects, results include a `disconnected` reason, and the agent can rejoin with `TeleportService` through `execute`.

Files go to `.macsploit/` in the working directory (with its own `.gitignore`), or to `$TMPDIR/macsploit-mcp/` when there's no project directory.

## Get started

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
   For clients that use an `mcpServers` JSON config (Pi's `~/.pi/agent/mcp.json`, Cursor, …):
   ```json
   { "mcpServers": { "macsploit": { "command": "/absolute/path/to/macsploit-mcp" } } }
   ```
3. Start your client, then open Roblox with MacSploit and join a game. The server installs the bridge on startup. If you were already in a game, rejoin once so autoexec runs it.

Long `execute` calls send a progress notification every 20s, which keeps clients like Pi waiting past their 60s default. Clients that don't reset their timeout on progress need a longer one; for Codex, set `tool_timeout_sec = 620`.

Only one game session works at a time. Several MCP clients can run at once, and they share the first server's connection.

## Repo layout

```
src/
├── main.rs             installs the bridge, serves MCP over stdio
├── mcp.rs              MCP server and tool registration
├── bridge.rs           owns the bridge port, or forwards to the process that does
├── broker.rs           job queue and client sessions
├── http.rs             bridge HTTP API (/poll, /result, /execute)
├── decompiler.rs       embedded Opiumware decompiler, run as a child process
├── install.rs          installs MCPBridge.lua into MacSploit
├── workspace.rs        .macsploit/ output paths
└── tools/              execute and dump_scripts
lua/
├── MCPBridge.lua       runs in the client and long-polls for jobs
└── collect_scripts.lua collects bytecode for dump_scripts
assets/                 demo media
```

## Credits

Decompilation uses [Opiumware](https://discord.gg/opiumware)'s decompiler, vendored in `vendor/` and embedded in the binary.

## License

MIT
