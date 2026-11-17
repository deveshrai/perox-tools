//! Starting rust-analyzer with the capabilities PeroxIDE uses.

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use serde_json::{Value, json};

use crate::client::{Client, Event};
use crate::uri;

/// A running, initialised rust-analyzer.
#[derive(Debug)]
pub struct RustAnalyzer {
    pub client: Client,
    /// Columns are chars (utf-32). If false the server chose utf-16.
    pub utf32: bool,
    /// Semantic token legend.
    pub token_types: Vec<String>,
    pub token_modifiers: Vec<String>,
}

pub const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
    "lifetime",
    "selfKeyword",
    "builtinType",
    "typeAlias",
    "union",
    "trait",
    "attribute",
    "constParameter",
    "label",
    "boolean",
    "generic",
];
pub const TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "definition",
    "readonly",
    "static",
    "deprecated",
    "abstract",
    "async",
    "modification",
    "documentation",
    "defaultLibrary",
    "mutable",
    "unsafe",
    "consuming",
    "callable",
    "constant",
    "library",
];

pub fn initialize_params(root: &Path) -> Value {
    let root_uri = uri::from_path(root);
    json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "PeroxIDE", "version": env!("CARGO_PKG_VERSION") },
        "rootUri": root_uri,
        "workspaceFolders": [{ "uri": root_uri, "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("project") }],
        "capabilities": {
            "general": { "positionEncodings": ["utf-32", "utf-16"] },
            "window": { "workDoneProgress": true },
            "workspace": { "configuration": false, "workspaceFolders": true, "applyEdit": false,
                "workspaceEdit": { "documentChanges": true } },
            "textDocument": {
                "synchronization": { "didSave": true, "dynamicRegistration": false },
                "completion": {
                    "contextSupport": true,
                    "completionItem": {
                        "snippetSupport": true,
                        "commitCharactersSupport": false,
                        "documentationFormat": ["markdown", "plaintext"],
                        "deprecatedSupport": true,
                        "preselectSupport": true,
                        "labelDetailsSupport": true,
                        "insertReplaceSupport": false,
                        // rust-analyzer offers auto-import items only if their `use`
                        // edits can be resolved lazily, when the item is accepted.
                        "resolveSupport": { "properties": ["additionalTextEdits"] },
                        "tagSupport": { "valueSet": [1] }
                    },
                    "completionItemKind": { "valueSet": (1..=25).collect::<Vec<u8>>() }
                },
                "hover": { "contentFormat": ["markdown", "plaintext"] },
                "signatureHelp": {
                    "contextSupport": true,
                    "signatureInformation": {
                        "documentationFormat": ["markdown", "plaintext"],
                        "parameterInformation": { "labelOffsetSupport": true },
                        "activeParameterSupport": true
                    }
                },
                "definition": { "linkSupport": false },
                "references": {},
                "rename": { "prepareSupport": false },
                "formatting": {},
                "inlayHint": { "resolveSupport": { "properties": [] } },
                "codeAction": {
                    "codeActionLiteralSupport": { "codeActionKind": { "valueSet": [
                        "", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports"
                    ] } },
                    "resolveSupport": { "properties": ["edit"] },
                    "dataSupport": true,
                    "isPreferredSupport": true
                },
                "publishDiagnostics": { "relatedInformation": false, "versionSupport": true },
                "semanticTokens": {
                    "requests": { "full": true, "range": false },
                    "tokenTypes": TOKEN_TYPES,
                    "tokenModifiers": TOKEN_MODIFIERS,
                    "formats": ["relative"],
                    "multilineTokenSupport": false,
                    "overlappingTokenSupport": false
                }
            },
            "experimental": { "serverStatusNotification": true }
        },
        "initializationOptions": {
            "check": { "command": "check" },
            "checkOnSave": true,
            "cargo": { "buildScripts": { "enable": true } },
            "procMacro": { "enable": true },
            "completion": {
                "autoimport": { "enable": true },
                "postfix": { "enable": true },
                "callable": { "snippets": "fill_arguments" },
                "privateEditable": { "enable": true },
                "limit": 200
            },
            "imports": { "granularity": { "group": "module" }, "prefix": "plain" },
            "semanticHighlighting": { "strings": { "enable": false } },
            "inlayHints": {
                "typeHints": { "enable": true, "hideClosureInitialization": true },
                "parameterHints": { "enable": true },
                "chainingHints": { "enable": true },
                "closingBraceHints": { "enable": false },
                "maxLength": 25
            },
            // Off: rust-analyzer's experimental diagnostics report false errors (e.g. an
            // "unresolved import" on `std` in a crate that compiles). Type errors come from
            // the compiler check on save instead.
            "diagnostics": { "experimental": { "enable": false } }
        }
    })
}

/// Starts rust-analyzer for `root` and completes the handshake. Blocks up to a minute:
/// call from a background thread.
pub fn start(
    program: &Path,
    root: &Path,
    env: &[(String, String)],
    wake: impl Fn() + Send + Sync + 'static,
) -> Result<(RustAnalyzer, Receiver<Event>), String> {
    let (client, events) = Client::spawn(program, &[], root, env, wake)
        .map_err(|e| format!("couldn't start rust-analyzer: {e}"))?;
    let init = client.request_blocking(
        "initialize",
        initialize_params(root),
        Duration::from_secs(60),
    )?;
    client.notify("initialized", json!({}));
    let caps = &init["capabilities"];
    let utf32 = caps["positionEncoding"].as_str() == Some("utf-32");
    let legend = &caps["semanticTokensProvider"]["legend"];
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok((
        RustAnalyzer {
            client,
            utf32,
            token_types: strings(&legend["tokenTypes"]),
            token_modifiers: strings(&legend["tokenModifiers"]),
        },
        events,
    ))
}
