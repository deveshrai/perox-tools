# perox-mcp

The built-in MCP server ([ADR 0006](../../docs/adr/0006-mcp-server.md)).

- `http.rs`: a deliberately small HTTP/1.1 reader and writer (one request per connection,
  `Content-Length` bodies, size and time limits). Standard library only.
- `server.rs`: the Streamable HTTP endpoint: bearer token, `Origin` check, sessions,
  JSON-RPC dispatch (`initialize`, `ping`, `tools/list`, `tools/call`).
- `tools.rs`: the four tools agents get: `list_commands`, `describe_command`,
  `execute` and `batch`. They run commands on the bus through the `Host` trait, which
  the app implements.

License: GPL-3.0-or-later.
