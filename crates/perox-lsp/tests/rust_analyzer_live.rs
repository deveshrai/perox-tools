//! Talks to a real rust-analyzer. Slow (it indexes std), so ignored by default:
//! `cargo test -p perox-lsp -- --ignored`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use perox_lsp::{Event, uri};
use serde_json::{Value, json};

fn rust_analyzer() -> Option<PathBuf> {
    let out = Command::new("rustup")
        .args(["which", "rust-analyzer"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

fn wait_for(events: &Receiver<Event>, id: i64, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(Event::Response { id: got, result }) =
            events.recv_timeout(Duration::from_millis(200))
            && got == id
        {
            return result.unwrap();
        }
    }
    panic!("no response to {id}");
}

#[test]
#[ignore]
fn completes_methods_and_auto_imports() {
    let Some(ra) = rust_analyzer() else {
        eprintln!("rust-analyzer not installed; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("perox-ra-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let src = "fn main() {\n    let v: Vec<u8> = Vec::new();\n    v.\n    let m = HashMa\n}\n";
    std::fs::write(dir.join("src/main.rs"), src).unwrap();

    let (ra, events) = perox_lsp::rust_analyzer::start(&ra, &dir, || {}).unwrap();
    assert!(ra.utf32, "rust-analyzer should accept utf-32 positions");
    let doc = uri::from_path(&dir.join("src/main.rs"));
    ra.client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": doc, "languageId": "rust", "version": 0, "text": src }}),
    );

    // Wait until indexing is done (serverStatus quiescent), up to 3 minutes.
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if let Ok(Event::Notification { method, params }) =
            events.recv_timeout(Duration::from_millis(500))
            && method == "experimental/serverStatus"
            && params["quiescent"] == true
        {
            break;
        }
    }

    let id = ra.client.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": doc },
            "position": { "line": 2, "character": 6 },
            "context": { "triggerKind": 2, "triggerCharacter": "." }
        }),
    );
    let result = wait_for(&events, id, Duration::from_secs(60));
    let items = result["items"]
        .as_array()
        .or(result.as_array())
        .unwrap()
        .clone();
    let labels: Vec<&str> = items.iter().filter_map(|i| i["label"].as_str()).collect();
    assert!(labels.iter().any(|l| l.starts_with("push")), "{labels:?}");

    // Auto-import candidates appear once std is indexed: ask until they do.
    let deadline = Instant::now() + Duration::from_secs(180);
    let hashmap = loop {
        let id = ra.client.request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": doc },
                "position": { "line": 3, "character": 18 },
                "context": { "triggerKind": 1 }
            }),
        );
        let result = wait_for(&events, id, Duration::from_secs(60));
        let items = result["items"]
            .as_array()
            .or(result.as_array())
            .cloned()
            .unwrap_or_default();
        if let Some(h) = items.iter().find(|i| {
            i["label"]
                .as_str()
                .is_some_and(|l| l.starts_with("HashMap"))
        }) {
            break h.clone();
        }
        assert!(
            Instant::now() < deadline,
            "HashMap never offered: {:?}",
            items
                .iter()
                .filter_map(|i| i["label"].as_str())
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_secs(2));
    };
    // The `use` line comes from resolving the item, as the IDE does on accept.
    let id = ra.client.request("completionItem/resolve", hashmap.clone());
    let resolved = wait_for(&events, id, Duration::from_secs(30));
    let edits = resolved["additionalTextEdits"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        edits.iter().any(|e| e["newText"]
            .as_str()
            .is_some_and(|t| t.contains("use std::collections::HashMap"))),
        "auto-import edit after resolve: {resolved}"
    );
    ra.client.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
