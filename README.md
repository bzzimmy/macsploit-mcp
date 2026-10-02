# macsploit-mcp

An MCP (Model Context Protocol) server for MacSploit that allows AI agents to interact with a live Roblox client without blocking the MacSploit application itself.

## Overview

MacSploit's local executor accepts a single TCP connection on port `5553`. If an external tool binds to this port, the MacSploit app can no longer use it. `macsploit-mcp` solves this by automatically installing an auto-execution script (`MCPBridge.lua`) into MacSploit. This bridge long-polls a local HTTP server (`127.0.0.1:8766`) for jobs, allowing the MCP server to execute Luau code without touching port `5553`.

## Features

- **Execute Luau:** Run arbitrary Luau scripts inside the live Roblox client with the full MacSploit (sUNC) API. Return values and output are captured and returned to the agent.
- **Decompile Scripts:** Automatically dump and decompile client scripts (LocalScripts, ModuleScripts) from the game instance tree into a local directory using a vendored Opiumware decompiler.
- **Large Output Handling:** Inline execution results are capped at around 3,000 tokens. Larger results are automatically truncated and saved to a local file for the agent to inspect.

## Provided Tools

- `execute(code, timeout_secs)`: Runs Luau code inside the connected Roblox client. Returns output and return values, saving large outputs to a file automatically.
- `dump_scripts(filter)`: Decompiles the game's client scripts into a local directory structure mirroring the instance tree (`.luau` files), making them easy to read and search.

## Installation & Usage

1. Build the project:
   ```sh
   cargo build --release
   ```
2. Run the server:
   ```sh
   ./target/release/macsploit-mcp
   ```
3. Open Roblox with MacSploit and join a game. The server will automatically install the necessary bridge script into `~/Documents/Macsploit Automatic Execution/`.
4. The MCP server connects via standard IO (`stdio`), exposing the tools to any compatible MCP client.
