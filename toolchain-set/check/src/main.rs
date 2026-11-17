//! Toolchain Set check: links every bundled crate and exercises the ones that can run
//! headless (no window, no network). CI runs this on macOS, Linux and Windows.
//! Exit code 0 and a final "OK" line mean the set is consistent on this platform.

use std::io::{Read, Write};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .init();
    // What `perox-ui` does at app start: select rustls' crypto provider (ring).
    let _ = rustls::crypto::ring::default_provider().install_default();

    check("serde_json, toml, ron, quick-xml, csv", formats)?;
    check("regex, unicode-segmentation, chrono", text_and_time)?;
    check("uuid, rand, sha2, base64", ids_and_hashes)?;
    check("flate2, zip", compression)?;
    check("image, resvg", images)?;
    check("fluent-bundle, unic-langid", i18n)?;
    check("rayon, crossbeam-channel, parking_lot", concurrency)?;
    check(
        "tokio, sqlx (sqlite), reqwest client + rustls/ring",
        async_db_net,
    )?;
    check("taffy", layout)?;
    check("directories", || {
        directories::ProjectDirs::from("dev", "PeroxIDE", "SetCheck")
            .ok_or_else(|| anyhow::anyhow!("no home directory"))?;
        Ok(())
    })?;
    link_only();

    println!("OK: Toolchain Set check passed");
    Ok(())
}

fn check(name: &str, f: impl FnOnce() -> anyhow::Result<()>) -> anyhow::Result<()> {
    f().map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
    tracing::info!("ok  {name}");
    log::debug!("log facade linked");
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Item {
    name: String,
    qty: u32,
}

fn formats() -> anyhow::Result<()> {
    let item = Item {
        name: "bolt".into(),
        qty: 3,
    };
    let back: Item = serde_json::from_str(&serde_json::to_string(&item)?)?;
    anyhow::ensure!(back == item);
    let back: Item = toml::from_str(&toml::to_string(&item)?)?;
    anyhow::ensure!(back == item);
    let back: Item = ron::from_str(&ron::to_string(&item)?)?;
    anyhow::ensure!(back == item);
    let back: Item = quick_xml::de::from_str(&quick_xml::se::to_string(&item)?)?;
    anyhow::ensure!(back == item);
    let mut w = csv::Writer::from_writer(vec![]);
    w.serialize(&item)?;
    let data = w.into_inner()?;
    let back: Item = csv::Reader::from_reader(&data[..])
        .deserialize()
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty csv"))??;
    anyhow::ensure!(back == item);
    Ok(())
}

fn text_and_time() -> anyhow::Result<()> {
    use unicode_segmentation::UnicodeSegmentation;
    anyhow::ensure!(regex::Regex::new(r"^\d{4}-\d{2}$")?.is_match("2026-11"));
    anyhow::ensure!("e\u{301}🇮🇳".graphemes(true).count() == 2);
    let d = chrono::NaiveDate::from_ymd_opt(2026, 11, 17).ok_or_else(|| anyhow::anyhow!("date"))?;
    anyhow::ensure!(d.format("%Y-%m-%d").to_string() == "2026-11-17");
    Ok(())
}

fn ids_and_hashes() -> anyhow::Result<()> {
    use base64::Engine;
    use sha2::Digest;
    anyhow::ensure!(uuid::Uuid::new_v4() != uuid::Uuid::now_v7());
    let n: u32 = rand::random();
    let _ = n;
    let digest = sha2::Sha256::digest(b"abc");
    let b64 = base64::engine::general_purpose::STANDARD.encode(digest);
    anyhow::ensure!(b64 == "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");
    Ok(())
}

fn compression() -> anyhow::Result<()> {
    let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
    gz.write_all(b"hello")?;
    let mut out = String::new();
    flate2::read::GzDecoder::new(&gz.finish()?[..]).read_to_string(&mut out)?;
    anyhow::ensure!(out == "hello");

    let mut buf = std::io::Cursor::new(vec![]);
    let mut zw = zip::ZipWriter::new(&mut buf);
    zw.start_file("a.txt", zip::write::SimpleFileOptions::default())?;
    zw.write_all(b"zip")?;
    zw.finish()?;
    let mut zr = zip::ZipArchive::new(buf)?;
    let mut s = String::new();
    zr.by_name("a.txt")?.read_to_string(&mut s)?;
    anyhow::ensure!(s == "zip");
    Ok(())
}

fn images() -> anyhow::Result<()> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
    let mut png = std::io::Cursor::new(vec![]);
    img.write_to(&mut png, image::ImageFormat::Png)?;
    let back = image::load_from_memory(png.get_ref())?;
    anyhow::ensure!(back.width() == 4);

    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="blue"/></svg>"#;
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default())?;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(8, 8).ok_or_else(|| anyhow::anyhow!("pixmap"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    anyhow::ensure!(pixmap.pixel(4, 4).map(|p| p.blue()) == Some(255));
    Ok(())
}

fn i18n() -> anyhow::Result<()> {
    let lang: unic_langid::LanguageIdentifier = "en-US".parse()?;
    let res = fluent_bundle::FluentResource::try_new("hello = Hello, { $name }!".to_owned())
        .map_err(|_| anyhow::anyhow!("fluent parse"))?;
    let mut bundle = fluent_bundle::FluentBundle::new(vec![lang]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(res)
        .map_err(|_| anyhow::anyhow!("fluent add"))?;
    let msg = bundle
        .get_message("hello")
        .and_then(|m| m.value())
        .ok_or_else(|| anyhow::anyhow!("no message"))?;
    let mut args = fluent_bundle::FluentArgs::new();
    args.set("name", "Rust");
    let mut errs = vec![];
    anyhow::ensure!(bundle.format_pattern(msg, Some(&args), &mut errs) == "Hello, Rust!");
    Ok(())
}

fn concurrency() -> anyhow::Result<()> {
    use rayon::prelude::*;
    anyhow::ensure!((1..=100u64).into_par_iter().sum::<u64>() == 5050);
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || tx.send(7));
    anyhow::ensure!(rx.recv()? == 7);
    let m = parking_lot::Mutex::new(1);
    *m.lock() += 1;
    anyhow::ensure!(*m.lock() == 2);
    Ok(())
}

fn async_db_net() -> anyhow::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(async {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await?;
        let (n,): (i64,) = sqlx::query_as("SELECT 40 + 2").fetch_one(&pool).await?;
        anyhow::ensure!(n == 42);
        // Building a client initialises rustls with its crypto provider: this is what
        // fails at runtime if two providers are enabled. No request is made.
        reqwest::Client::builder().build()?;
        anyhow::Ok(())
    })?;
    reqwest::blocking::Client::builder().build()?;
    Ok(())
}

fn layout() -> anyhow::Result<()> {
    use taffy::prelude::*;
    let mut t: TaffyTree<()> = TaffyTree::new();
    let child = t.new_leaf(Style {
        flex_grow: 1.0,
        ..Default::default()
    })?;
    let root = t.new_with_children(
        Style {
            size: Size {
                width: length(200.0),
                height: length(100.0),
            },
            ..Default::default()
        },
        &[child],
    )?;
    t.compute_layout(root, Size::MAX_CONTENT)?;
    anyhow::ensure!(t.layout(child)?.size.width == 200.0);
    Ok(())
}

/// GUI and OS-integration crates need a window, a display or user interaction.
/// Naming their types proves they compile and link on this platform.
fn link_only() {
    let _: Option<eframe::NativeOptions> = None;
    let _: Option<egui::Context> = None;
    let _: Option<egui_extras::TableBuilder<'static>> = None;
    let _: Option<egui_plot::Plot<'static>> = None;
    let _: Option<egui_tiles::Tree<()>> = None;
    let _: Option<rfd::FileDialog> = None;
    let _: Option<arboard::Clipboard> = None;
    let _ = |p: &str| open::that(p);
    let _: Option<thiserror_marker::E> = None;
}

mod thiserror_marker {
    #[derive(Debug, thiserror::Error)]
    #[error("marker")]
    pub struct E;
}
