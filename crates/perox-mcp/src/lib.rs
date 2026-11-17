//! PeroxIDE's MCP server. See the crate README and `docs/adr/0006-mcp-server.md`.

mod http;
mod server;
mod tools;

pub use server::{PROTOCOL_VERSIONS, Server, ServerConfig, Stats};
pub use tools::Host;

/// A fresh random access token: 64 hex characters (256 bits).
pub fn generate_token() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The command that starts Claude Code connected to this server for one session.
pub fn claude_session_command(url: &str, token: &str) -> String {
    let config = serde_json::json!({
        "mcpServers": { "peroxide": {
            "type": "http",
            "url": url,
            "headers": { "Authorization": format!("Bearer {token}") }
        }}
    });
    format!("claude --mcp-config '{config}'")
}

/// The command that adds this server to Claude Code permanently.
pub fn claude_add_command(url: &str, token: &str) -> String {
    format!(
        "claude mcp add --transport http peroxide {url} --header \"Authorization: Bearer {token}\""
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokens_are_64_hex_and_differ() {
        let (a, b) = (super::generate_token(), super::generate_token());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn session_command_embeds_url_and_token() {
        let c = super::claude_session_command("http://127.0.0.1:27890/mcp", "abc");
        assert!(c.starts_with("claude --mcp-config '{"));
        assert!(c.contains(r#""url":"http://127.0.0.1:27890/mcp""#));
        assert!(c.contains("Bearer abc"));
    }
}
