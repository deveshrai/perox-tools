//! Stock data from free services that need no API key.
//!
//! | What | Where |
//! |---|---|
//! | Quote and daily (or weekly) prices | Yahoo Finance chart API |
//! | Sector and industry | Yahoo Finance search |
//! | The company's SEC id (CIK) | SEC EDGAR's search index |
//! | Address, incorporation, filings | SEC EDGAR submissions |
//!
//! Fetching and parsing are separate: the `parse_*` functions take the services' JSON
//! and are tested offline against saved responses (`tests/fixtures`), and [`fetch`]
//! does the network calls, on whatever thread it's called from (never the UI's).

use std::time::Duration;

use serde_json::Value;

/// Sent with every request. SEC asks automated clients to say who they are.
const USER_AGENT: &str = "PeroxIDE-example-stock-dashboard/0.1";

/// The price history ranges the window offers, in its combo box's order:
/// (label, Yahoo `range`, Yahoo `interval`).
pub const RANGES: &[(&str, &str, &str)] = &[
    ("1 month", "1mo", "1d"),
    ("3 months", "3mo", "1d"),
    ("6 months", "6mo", "1d"),
    ("1 year", "1y", "1d"),
    ("5 years", "5y", "1wk"),
];

/// One trading day (or week): Unix time and prices.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u64,
}

/// The latest trading figures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Quote {
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub exchange: String,
    pub price: f64,
    pub previous_close: f64,
    pub day_low: f64,
    pub day_high: f64,
    pub year_low: f64,
    pub year_high: f64,
    pub volume: u64,
}

/// Who the company is, as far as the free sources say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Company {
    pub sector: String,
    pub industry: String,
    /// SEC's id, zero-padded to 10 digits; empty if SEC doesn't know the ticker (funds,
    /// foreign listings).
    pub cik: String,
    pub sec_name: String,
    pub sic: String,
    pub address: String,
    pub incorporated: String,
    /// `MMDD`, as SEC writes it.
    pub fiscal_year_end: String,
    pub website: String,
    /// The latest annual report (10-K, or 20-F for foreign filers).
    pub annual_report: Option<Filing>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Filing {
    pub form: String,
    pub date: String,
    pub url: String,
}

/// Everything shown for one ticker.
#[derive(Debug, Clone, PartialEq)]
pub struct StockData {
    pub quote: Quote,
    pub bars: Vec<Bar>,
    pub company: Company,
}

/// Links the window offers for a ticker.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Links {
    pub yahoo: String,
    pub sec: Option<String>,
    pub annual_report: Option<String>,
    pub website: Option<String>,
}

impl StockData {
    pub fn links(&self) -> Links {
        let c = &self.company;
        Links {
            yahoo: format!("https://finance.yahoo.com/quote/{}", self.quote.symbol),
            sec: (!c.cik.is_empty()).then(|| {
                format!(
                    "https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK={}&owner=include&count=40",
                    c.cik
                )
            }),
            annual_report: c.annual_report.as_ref().map(|f| f.url.clone()),
            website: (!c.website.is_empty()).then(|| {
                if c.website.starts_with("http") {
                    c.website.clone()
                } else {
                    format!("https://{}", c.website)
                }
            }),
        }
    }
}

/// Cleans up what was typed: `" aapl "` → `"AAPL"`. `None` if it can't be a ticker.
pub fn normalize_ticker(text: &str) -> Option<String> {
    let t = text.trim().to_uppercase();
    let ok = !t.is_empty()
        && t.len() <= 12
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '^' | '='));
    ok.then_some(t)
}

/// Fetches everything for `ticker` (already normalized). Blocking: call it from a
/// background thread. The quote and prices are required; company details are best
/// effort, so a fund or foreign listing still shows its prices.
pub fn fetch(ticker: &str, range: usize) -> Result<StockData, String> {
    let agent = agent();
    let get = |url: &str| get_json(&agent, ticker, url);

    let (_, yahoo_range, interval) = RANGES[range.min(RANGES.len() - 1)];
    let chart = get(&format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{ticker}?range={yahoo_range}&interval={interval}"
    ))?;
    let (quote, bars) = parse_chart(&chart)?;

    let mut company = Company::default();
    if let Ok(search) = get(&format!(
        "https://query1.finance.yahoo.com/v1/finance/search?q={ticker}&quotesCount=5&newsCount=0"
    )) {
        let (sector, industry) = parse_search(&search, ticker);
        company.sector = sector;
        company.industry = industry;
    }
    if let Ok(found) = get(&format!(
        "https://efts.sec.gov/LATEST/search-index?keysTyped={ticker}"
    )) && let Some(cik) = parse_sec_search(&found, ticker)
        && let Ok(sub) = get(&format!("https://data.sec.gov/submissions/CIK{cik}.json"))
    {
        let sector = std::mem::take(&mut company.sector);
        let industry = std::mem::take(&mut company.industry);
        company = parse_submissions(&sub);
        company.sector = sector;
        company.industry = industry;
    }
    Ok(StockData {
        quote,
        bars,
        company,
    })
}

/// The latest quote, for the watchlist: the chart API over the last five days.
pub fn fetch_quote(ticker: &str) -> Result<Quote, String> {
    let agent = agent();
    let chart = get_json(
        &agent,
        ticker,
        &format!("https://query1.finance.yahoo.com/v8/finance/chart/{ticker}?range=5d&interval=1d"),
    )?;
    parse_chart(&chart).map(|(q, _)| q)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into()
}

/// GETs `url` and reads its JSON, with errors a person can read.
fn get_json(agent: &ureq::Agent, ticker: &str, url: &str) -> Result<Value, String> {
    let text = agent
        .get(url)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => format!("No data for {ticker}."),
            e => format!("Couldn't reach {}: {e}", host(url)),
        })?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Couldn't read the answer from {}: {e}", host(url)))?;
    serde_json::from_str(&text).map_err(|e| format!("{} sent something unexpected: {e}", host(url)))
}

fn host(url: &str) -> &str {
    url.split('/').nth(2).unwrap_or(url)
}

/// The quote and price bars from Yahoo's chart API. Days without a close (holidays in
/// the middle of a range, a trading day that hasn't closed) are skipped.
pub fn parse_chart(v: &Value) -> Result<(Quote, Vec<Bar>), String> {
    if let Some(desc) = v["chart"]["error"]["description"].as_str() {
        return Err(desc.to_owned());
    }
    let r = &v["chart"]["result"][0];
    if r.is_null() {
        return Err("No such ticker.".into());
    }
    let m = &r["meta"];
    let f = |k: &str| m[k].as_f64().unwrap_or(0.0);
    let s = |k: &str| m[k].as_str().unwrap_or_default().to_owned();
    let mut quote = Quote {
        symbol: s("symbol"),
        name: m["longName"]
            .as_str()
            .or(m["shortName"].as_str())
            .unwrap_or_default()
            .to_owned(),
        currency: s("currency"),
        exchange: s("fullExchangeName"),
        price: f("regularMarketPrice"),
        previous_close: m["previousClose"]
            .as_f64()
            .unwrap_or(f("chartPreviousClose")),
        day_low: f("regularMarketDayLow"),
        day_high: f("regularMarketDayHigh"),
        year_low: f("fiftyTwoWeekLow"),
        year_high: f("fiftyTwoWeekHigh"),
        volume: m["regularMarketVolume"].as_u64().unwrap_or(0),
    };

    let times = r["timestamp"].as_array().cloned().unwrap_or_default();
    let q = &r["indicators"]["quote"][0];
    let col = |k: &str, i: usize| q[k][i].as_f64();
    let bars: Vec<Bar> = times
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            Some(Bar {
                time: t.as_i64()?,
                close: col("close", i)?,
                open: col("open", i).unwrap_or(f64::NAN),
                high: col("high", i).unwrap_or(f64::NAN),
                low: col("low", i).unwrap_or(f64::NAN),
                volume: q["volume"][i].as_u64().unwrap_or(0),
            })
        })
        .collect();
    // For a range longer than a day, "previous close" in the meta is the close before
    // the range; the day's change is against the bar before the last.
    if bars.len() >= 2 {
        quote.previous_close = bars[bars.len() - 2].close;
    }
    Ok((quote, bars))
}

/// Sector and industry from Yahoo's search, for the quote whose symbol is `ticker`.
pub fn parse_search(v: &Value, ticker: &str) -> (String, String) {
    let quotes = v["quotes"].as_array().cloned().unwrap_or_default();
    let q = quotes
        .iter()
        .find(|q| q["symbol"].as_str() == Some(ticker))
        .cloned()
        .unwrap_or(Value::Null);
    let s = |k: &str| q[k].as_str().unwrap_or_default().to_owned();
    (s("sectorDisp"), s("industryDisp"))
}

/// The CIK (zero-padded to 10 digits) of the company whose tickers include `ticker`.
pub fn parse_sec_search(v: &Value, ticker: &str) -> Option<String> {
    let hits = v["hits"]["hits"].as_array()?;
    hits.iter().find_map(|h| {
        let tickers = h["_source"]["tickers"].as_str()?;
        let matches = tickers
            .split(',')
            .any(|t| t.trim().eq_ignore_ascii_case(ticker));
        let id = h["_id"].as_str()?;
        (matches && id.chars().all(|c| c.is_ascii_digit())).then(|| format!("{id:0>10}"))
    })
}

/// Company details from SEC's submissions file.
pub fn parse_submissions(v: &Value) -> Company {
    let s = |k: &str| v[k].as_str().unwrap_or_default().trim().to_owned();
    let cik = format!("{:0>10}", s("cik"));
    let a = &v["addresses"]["business"];
    let part = |k: &str| a[k].as_str().unwrap_or_default().trim().to_owned();
    let address = [
        part("street1"),
        part("street2"),
        [part("city"), part("stateOrCountry"), part("zipCode")]
            .iter()
            .filter(|p| !p.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect::<Vec<_>>()
    .join(", ");

    let recent = &v["filings"]["recent"];
    let list = |k: &str| recent[k].as_array().cloned().unwrap_or_default();
    let (forms, dates, accessions, docs) = (
        list("form"),
        list("filingDate"),
        list("accessionNumber"),
        list("primaryDocument"),
    );
    // Filings are newest first.
    let annual_report = forms
        .iter()
        .position(|f| matches!(f.as_str(), Some("10-K" | "20-F")))
        .and_then(|i| {
            let accession = accessions.get(i)?.as_str()?.replace('-', "");
            let doc = docs.get(i)?.as_str()?;
            Some(Filing {
                form: forms[i].as_str()?.to_owned(),
                date: dates.get(i)?.as_str()?.to_owned(),
                url: format!(
                    "https://www.sec.gov/Archives/edgar/data/{}/{accession}/{doc}",
                    cik.trim_start_matches('0')
                ),
            })
        });

    Company {
        sector: String::new(),
        industry: String::new(),
        cik,
        sec_name: s("name"),
        sic: s("sicDescription"),
        address: titlecase(&address),
        incorporated: s("stateOfIncorporation"),
        fiscal_year_end: s("fiscalYearEnd"),
        website: s("website"),
        annual_report,
    }
}

/// SEC writes addresses in capitals: `ONE APPLE PARK WAY` → `One Apple Park Way`.
/// Two-letter words (state codes) stay as they are.
fn titlecase(text: &str) -> String {
    text.split(' ')
        .map(|w| {
            if w.len() <= 2 || w.chars().any(|c| c.is_ascii_digit()) {
                w.to_owned()
            } else {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_string() + &c.as_str().to_lowercase())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn tickers_are_cleaned_up() {
        assert_eq!(normalize_ticker(" aapl "), Some("AAPL".into()));
        assert_eq!(normalize_ticker("brk-b"), Some("BRK-B".into()));
        assert_eq!(normalize_ticker("^gspc"), Some("^GSPC".into()));
        assert_eq!(normalize_ticker(""), None);
        assert_eq!(normalize_ticker("not a ticker"), None);
    }

    #[test]
    fn reads_the_quote_and_bars() {
        let (q, bars) = parse_chart(&fixture("chart.json")).unwrap();
        assert_eq!(q.symbol, "AAPL");
        assert_eq!(q.name, "Apple Inc.");
        assert_eq!(q.currency, "USD");
        assert!(q.price > 0.0 && q.year_high >= q.year_low);
        assert!(!bars.is_empty());
        assert!(bars.windows(2).all(|w| w[0].time < w[1].time));
        assert_eq!(q.previous_close, bars[bars.len() - 2].close);
    }

    #[test]
    fn a_chart_error_is_reported() {
        let v = serde_json::json!({ "chart": { "result": null, "error": { "code": "Not Found", "description": "No data found, symbol may be delisted" } } });
        assert_eq!(
            parse_chart(&v).unwrap_err(),
            "No data found, symbol may be delisted"
        );
    }

    #[test]
    fn reads_sector_and_industry() {
        let (sector, industry) = parse_search(&fixture("search.json"), "AAPL");
        assert_eq!(sector, "Technology");
        assert_eq!(industry, "Consumer Electronics");
        assert_eq!(
            parse_search(&fixture("search.json"), "ZZZZ"),
            (String::new(), String::new())
        );
    }

    #[test]
    fn finds_the_cik_by_exact_ticker() {
        assert_eq!(
            parse_sec_search(&fixture("sec_search.json"), "AAPL"),
            Some("0000320193".into())
        );
        assert_eq!(parse_sec_search(&fixture("sec_search.json"), "AAP"), None);
    }

    #[test]
    fn reads_company_details_and_the_annual_report() {
        let c = parse_submissions(&fixture("submissions.json"));
        assert_eq!(c.cik, "0000320193");
        assert_eq!(c.sec_name, "Apple Inc.");
        assert_eq!(c.address, "One Apple Park Way, Cupertino CA 95014");
        assert_eq!(c.incorporated, "CA");
        let report = c.annual_report.unwrap();
        assert_eq!(report.form, "10-K");
        assert!(
            report
                .url
                .starts_with("https://www.sec.gov/Archives/edgar/data/320193/"),
            "{}",
            report.url
        );
    }

    #[test]
    fn links_need_what_they_point_at() {
        let data = StockData {
            quote: Quote {
                symbol: "AAPL".into(),
                ..Default::default()
            },
            bars: Vec::new(),
            company: Company {
                website: "apple.com".into(),
                ..Default::default()
            },
        };
        let l = data.links();
        assert_eq!(l.yahoo, "https://finance.yahoo.com/quote/AAPL");
        assert_eq!(l.sec, None, "no CIK, no SEC link");
        assert_eq!(l.website.as_deref(), Some("https://apple.com"));
    }
}
