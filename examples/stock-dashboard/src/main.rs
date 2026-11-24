//! Stock Dashboard: a watchlist of stocks with live quotes; pick one to see its price
//! chart, the day's numbers, its daily history and company details.
//!
//! A PeroxIDE sample, the restyled sibling of Stock Viewer. The window is
//! `ui/main_window.slint`, made in PeroxIDE's form designer out of the components in
//! `ui/parts/` (Card, StatTile, Segmented, WatchRow), also made there. Each `callback`
//! the window declares is a method of `MainWindowHandlers` below. `BUILDING.md` tells how
//! the app was built.

mod chart;
mod data;
mod format;

perox_ui::include_ui!();

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use slint::{ComponentHandle, Model, ModelRc, SharedString, StandardListViewItem, VecModel};

/// The watchlist: symbol and a short name to show until the quote arrives.
const WATCHLIST: &[(&str, &str)] = &[
    ("AAPL", "Apple"),
    ("MSFT", "Microsoft"),
    ("NVDA", "NVIDIA"),
    ("GOOGL", "Alphabet"),
    ("AMZN", "Amazon"),
    ("META", "Meta Platforms"),
    ("TSLA", "Tesla"),
    ("^GSPC", "S&P 500"),
];

/// The range picker starts on its third choice, 6M (`data::RANGES` is in the same order).
const START_RANGE: usize = 2;

/// What the handlers share with the background lookups.
#[derive(Debug, Default)]
struct App {
    /// The stock on show, and the range it's shown over.
    ticker: Option<String>,
    range: usize,
    /// Its links (filled in by the lookup that showed it).
    links: Arc<Mutex<data::Links>>,
    /// Bumped by each lookup; a lookup that finishes after a newer one started is
    /// thrown away, so a slow answer never replaces a newer one.
    latest: Arc<AtomicU64>,
}

impl App {
    /// Looks `ticker` up over the chosen range without blocking the window: the network
    /// calls run on a thread, and the result comes back on the UI thread.
    fn look_up(&mut self, ui: &MainWindow, ticker: String) {
        let range = self.range;
        self.ticker = Some(ticker.clone());
        let id = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        ui.set_busy(true);
        ui.set_status(format!("Looking up {ticker}…").into());
        // Highlight it in the watchlist if it's there.
        let at = WATCHLIST.iter().position(|(s, _)| *s == ticker);
        ui.set_selected(at.map_or(-1, |i| i as i32));

        let (window, latest, links) = (ui.as_weak(), self.latest.clone(), self.links.clone());
        std::thread::spawn(move || {
            let result = data::fetch(&ticker, range);
            if latest.load(Ordering::SeqCst) != id {
                return; // a newer lookup has started
            }
            if let Ok(found) = &result {
                *links.lock().unwrap_or_else(PoisonError::into_inner) = found.links();
            }
            let _ = window.upgrade_in_event_loop(move |ui| {
                ui.set_busy(false);
                match result {
                    Ok(found) => show(&ui, &found, range),
                    Err(e) => ui.set_status(e.into()),
                }
            });
        });
    }
}

impl MainWindowHandlers for App {
    fn about_clicked(&mut self, ui: &MainWindow) {
        if let Ok(about) = AboutDialog::new() {
            let _ = about.exec(Some(ui.window()), |_| {});
        }
    }

    fn watch_clicked(&mut self, ui: &MainWindow, index: i32) {
        if let Some((symbol, _)) = WATCHLIST.get(index as usize) {
            self.look_up(ui, (*symbol).to_owned());
        }
    }

    /// Return in the search box.
    fn search_accepted(&mut self, ui: &MainWindow, text: SharedString) {
        match data::normalize_ticker(&text) {
            Some(t) => self.look_up(ui, t),
            None => ui.set_status("Type a ticker symbol, like AAPL or MSFT.".into()),
        }
    }

    /// A new range: the same stock again over it.
    fn range_picked(&mut self, ui: &MainWindow, index: i32) {
        self.range = (index.max(0) as usize).min(data::RANGES.len() - 1);
        if let Some(t) = self.ticker.clone() {
            self.look_up(ui, t);
        }
    }

    /// Yahoo Finance, SEC filings, the annual report, the company's website.
    fn link_clicked(&mut self, ui: &MainWindow, index: i32) {
        let links = self
            .links
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let url = match index {
            0 => Some(links.yahoo).filter(|u| !u.is_empty()),
            1 => links.sec,
            2 => links.annual_report,
            _ => links.website,
        };
        if let Some(url) = url
            && let Err(e) = webbrowser::open(&url)
        {
            ui.set_status(format!("Couldn't open the browser: {e}").into());
        }
    }
}

/// Puts a lookup's result into the window.
fn show(ui: &MainWindow, d: &data::StockData, range: usize) {
    let q = &d.quote;
    let c = &d.company;
    let text = |s: String| SharedString::from(s);

    ui.set_company(text(q.name.clone()));
    let sector = [c.sector.as_str(), c.industry.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let meta: Vec<&str> = [q.symbol.as_str(), q.exchange.as_str(), sector.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    ui.set_meta(text(meta.join("  ·  ")));
    ui.set_price(text(format!("{} {}", format::money(q.price), q.currency)));
    ui.set_change(text(format::change(q.price, q.previous_close)));
    ui.set_up(q.price >= q.previous_close);

    // The day's numbers.
    let open = d.bars.last().map(|b| b.open).unwrap_or(f64::NAN);
    ui.set_open_text(text(format::money(open)));
    ui.set_day_range(text(format!(
        "{} – {}",
        format::money(q.day_low),
        format::money(q.day_high)
    )));
    ui.set_year_range(text(format!(
        "{} – {}",
        format::money(q.year_low),
        format::money(q.year_high)
    )));
    ui.set_volume(text(format::volume(q.volume)));

    // Chart, with the range's low and high under it.
    let closes: Vec<f64> = d.bars.iter().map(|b| b.close).collect();
    ui.set_chart_line(chart::line_commands(&closes).into());
    ui.set_chart_area(chart::area_commands(&closes).into());
    let lo = closes.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = closes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (label, _, _) = data::RANGES[range.min(data::RANGES.len() - 1)];
    if lo.is_finite() && hi.is_finite() {
        ui.set_chart_low(text(format!("Low {} ", format::money(lo))));
        ui.set_chart_high(text(format!("High {} over {label}", format::money(hi))));
    }

    // History, newest first.
    let rows: Vec<ModelRc<StandardListViewItem>> = d
        .bars
        .iter()
        .rev()
        .map(|b| {
            let cells: Vec<StandardListViewItem> = [
                format::date(b.time),
                format::money(b.open),
                format::money(b.high),
                format::money(b.low),
                format::money(b.close),
                format::volume(b.volume),
            ]
            .into_iter()
            .map(|s| StandardListViewItem::from(SharedString::from(s)))
            .collect();
            ModelRc::new(VecModel::from(cells))
        })
        .collect();
    ui.set_history_rows(ModelRc::new(VecModel::from(rows)));

    // Company: one line per fact a source gave.
    let mut facts: Vec<String> = Vec::new();
    for (label, value) in [
        ("Address", c.address.clone()),
        ("Incorporated in", c.incorporated.clone()),
        ("Fiscal year ends", format::month_day(&c.fiscal_year_end)),
    ] {
        if !value.is_empty() {
            facts.push(format!("{label}: {value}"));
        }
    }
    facts.push(match (&c.cik, &c.annual_report) {
        (cik, Some(r)) if !cik.is_empty() => {
            format!("SEC CIK {cik} · last {} filed {}", r.form, r.date)
        }
        (cik, None) if !cik.is_empty() => format!("SEC CIK {cik}"),
        _ => "Not an SEC filer, so there are no company filings to show.".into(),
    });
    ui.set_details(text(facts.join("\n")));

    // Links: each button only when there's somewhere to go.
    let links = d.links();
    let flags = [
        true,
        links.sec.is_some(),
        links.annual_report.is_some(),
        links.website.is_some(),
    ];
    ui.set_links(ModelRc::new(VecModel::from(flags.to_vec())));

    ui.set_status(text(format!(
        "{} prices · Yahoo Finance and SEC EDGAR",
        d.bars.len()
    )));
}

/// Fills the watchlist with names, then fetches each quote on a thread and updates its
/// row as it arrives.
fn load_watchlist(ui: &MainWindow) {
    let items: Vec<WatchItem> = WATCHLIST
        .iter()
        .map(|(symbol, name)| WatchItem {
            symbol: (*symbol).into(),
            company: (*name).into(),
            price: "…".into(),
            change: SharedString::new(),
            up: true,
        })
        .collect();
    ui.set_watchlist(ModelRc::new(VecModel::from(items)));
    let window = ui.as_weak();
    std::thread::spawn(move || {
        for (i, (symbol, name)) in WATCHLIST.iter().enumerate() {
            let Ok(q) = data::fetch_quote(symbol) else {
                continue;
            };
            let item = WatchItem {
                symbol: (*symbol).into(),
                company: (*name).into(),
                price: format::money(q.price).into(),
                change: format::change(q.price, q.previous_close).into(),
                up: q.price >= q.previous_close,
            };
            let _ =
                window.upgrade_in_event_loop(move |ui| ui.get_watchlist().set_row_data(i, item));
        }
    });
}

/// Starts the window on the first stock of the watchlist (or `stock-dashboard TICKER`).
fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    load_watchlist(&ui);
    let mut app = App {
        range: START_RANGE,
        ..Default::default()
    };
    let first = std::env::args()
        .nth(1)
        .and_then(|t| data::normalize_ticker(&t))
        .unwrap_or_else(|| WATCHLIST[0].0.to_owned());
    app.look_up(&ui, first);
    ui.connect(app);
    ui.run()
}
