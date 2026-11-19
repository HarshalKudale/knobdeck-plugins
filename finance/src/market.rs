//! Quotes. Crypto from CoinGecko's public API (no key; the user's free Demo key, if saved, raises
//! the limit). Stocks, ETFs, indices and forex from Yahoo Finance's chart endpoint (no key), with
//! Finnhub's quote endpoint as the fallback when the user saved a Finnhub key.
//!
//! Every reply is cached and shared by all keys: CoinGecko is asked once per currency for every
//! coin any key shows (two batched calls), at most every two minutes; Yahoo once a minute per
//! symbol. On a rate limit the plugin backs off and keeps showing the last prices.

use knobdeck_sdk::{Error, Result, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const CRYPTO_TTL: Duration = Duration::from_secs(120);
pub const STOCK_TTL: Duration = Duration::from_secs(60);
/// The fewest seconds between two CoinGecko refreshes of coins already shown…
const GECKO_GAP: Duration = Duration::from_secs(20);
/// …and when a key asks for a coin not fetched yet.
const GECKO_NEW_GAP: Duration = Duration::from_secs(5);
/// A coin no key asked for in this long is no longer fetched.
const WANTED_FOR: Duration = Duration::from_secs(600);
/// Stale prices are still shown this long when the source fails.
const STALE_OK: Duration = Duration::from_secs(1800);
const UA: &str = "Mozilla/5.0 (compatible; KnobDeck-Finance/1.0)";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quote {
    /// Short name on the key: "BTC", "AAPL", "S&P 500".
    pub symbol: String,
    pub name: String,
    pub price: f64,
    /// Change over the chart's span (24 h / day, or 7 days / 5 trading days), in %.
    pub change: Option<f64>,
    /// ISO code, upper case ("USD"); empty when the source doesn't say.
    pub currency: String,
    /// Prices over the span, oldest first (may be empty).
    pub points: Vec<f64>,
    /// Decimals to show, when the source says (Yahoo's `priceHint`: 4 for forex).
    pub decimals: Option<usize>,
    /// The page a press opens.
    pub url: String,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .user_agent(UA)
        .build()
}

/// Percent-encode a query or path component.
pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ── CoinGecko ──

#[derive(Clone, Debug)]
struct Coin {
    id: String,
    symbol: String,
    name: String,
    price: f64,
    ch24: Option<f64>,
    ch7d: Option<f64>,
    /// 7 days, hourly (168 points).
    spark: Vec<f64>,
}

#[derive(Default)]
struct Gecko {
    /// (currency, entry as typed, lower case) → when a key last asked for it.
    wanted: HashMap<(String, String), Instant>,
    /// (currency, entry) → coin; `None` once CoinGecko said there is no such coin.
    coins: HashMap<(String, String), (Instant, Option<Coin>)>,
    /// Entry → CoinGecko id, once found: later refreshes need only the by-id call.
    ids: HashMap<String, String>,
    /// Per currency: the last refresh (tried).
    tried: HashMap<String, Instant>,
    backoff: Option<Instant>,
}

static GECKO: Mutex<Option<Gecko>> = Mutex::new(None);

/// A short entry without a dash ("btc", "sol") is looked up as a symbol first (the coin with the
/// biggest market cap wins), anything else ("bitcoin", "usd-coin") as a CoinGecko id first.
fn symbol_first(entry: &str) -> bool {
    entry.len() <= 6 && !entry.contains('-')
}

fn coin_from(v: &Value) -> Option<Coin> {
    Some(Coin {
        id: v["id"].as_str()?.to_string(),
        symbol: v["symbol"].as_str().unwrap_or("").to_uppercase(),
        name: v["name"].as_str().unwrap_or("").to_string(),
        price: v["current_price"].as_f64()?,
        ch24: v["price_change_percentage_24h_in_currency"]
            .as_f64()
            .or(v["price_change_percentage_24h"].as_f64()),
        ch7d: v["price_change_percentage_7d_in_currency"].as_f64(),
        spark: v["sparkline_in_7d"]["price"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default(),
    })
}

fn gecko_get(query: &[(&str, &str)]) -> std::result::Result<Vec<Value>, (u16, String)> {
    let mut req = agent().get("https://api.coingecko.com/api/v3/coins/markets");
    for (k, v) in query {
        req = req.query(k, v);
    }
    let key = gecko_key();
    if !key.is_empty() {
        req = req.set("x-cg-demo-api-key", &key);
    }
    match req.call() {
        Ok(r) => r
            .into_json::<Vec<Value>>()
            .map_err(|e| (0, format!("CoinGecko sent an invalid reply: {e}"))),
        Err(ureq::Error::Status(code, r)) => Err((
            code,
            r.into_string()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect(),
        )),
        Err(e) => Err((0, format!("CoinGecko unavailable ({e})"))),
    }
}

fn gecko_key() -> String {
    knobdeck_sdk::secrets::env_or_stored("COINGECKO_API_KEY")
}

/// Ask CoinGecko for every wanted coin in `vs`: one call by id, one by symbol.
fn gecko_fetch(g: &mut Gecko, vs: &str) -> Result<()> {
    let now = Instant::now();
    g.wanted.retain(|_, t| now.duration_since(*t) < WANTED_FOR);
    let entries: Vec<String> = g
        .wanted
        .keys()
        .filter(|(c, _)| c == vs)
        .map(|(_, e)| e.clone())
        .take(250)
        .collect();
    if entries.is_empty() {
        return Ok(());
    }
    g.tried.insert(vs.to_string(), now);
    let mut ids: Vec<String> = entries
        .iter()
        .map(|e| g.ids.get(e).unwrap_or(e).clone())
        .collect();
    ids.sort();
    ids.dedup();
    let unresolved: Vec<String> = entries
        .iter()
        .filter(|e| !g.ids.contains_key(*e))
        .cloned()
        .collect();
    let (ids, symbols) = (ids.join(","), unresolved.join(","));
    let common = [
        ("vs_currency", vs),
        ("sparkline", "true"),
        ("price_change_percentage", "24h,7d"),
        ("per_page", "250"),
    ];
    let mut by_id = HashMap::new();
    let mut by_symbol: HashMap<String, Coin> = HashMap::new();
    let mut calls: Vec<(&str, &str)> = vec![("ids", &ids)];
    if !unresolved.is_empty() {
        calls.push(("symbols", &symbols));
    }
    for (param, value) in calls {
        let mut q = common.to_vec();
        q.push((param, value));
        if param == "symbols" {
            q.push(("include_tokens", "top"));
        }
        match gecko_get(&q) {
            Ok(rows) => {
                for c in rows.iter().filter_map(coin_from) {
                    if param == "ids" {
                        by_id.insert(c.id.clone(), c);
                    } else {
                        by_symbol.entry(c.symbol.to_lowercase()).or_insert(c);
                    }
                }
            }
            Err((429, _)) => {
                g.backoff = Some(now + Duration::from_secs(60));
                return Err(Error::msg("CoinGecko's free limit was reached; showing the last prices, trying again in a minute"));
            }
            Err((400, body)) if body.contains("currency") => {
                return Err(Error::msg(format!(
                    "CoinGecko doesn't know the currency \"{vs}\""
                )))
            }
            Err((code, msg)) if code == 401 || code == 403 => {
                return Err(Error::msg(format!(
                    "CoinGecko refused the request ({code}): check the API key in setup. {msg}"
                )))
            }
            Err((code, msg)) => {
                return Err(Error::msg(if code == 0 {
                    msg
                } else {
                    format!("CoinGecko error {code}: {msg}")
                }))
            }
        }
    }
    for e in entries {
        let found = match g.ids.get(&e) {
            Some(id) => by_id.get(id),
            None if symbol_first(&e) => by_symbol.get(&e).or(by_id.get(&e)),
            None => by_id.get(&e).or(by_symbol.get(&e)),
        }
        .cloned();
        if let Some(c) = &found {
            g.ids.insert(e.clone(), c.id.clone());
        }
        g.coins.insert((vs.to_string(), e), (now, found));
    }
    Ok(())
}

fn norm_vs(vs: &str) -> String {
    match vs.trim().to_lowercase() {
        v if v.is_empty() => "usd".to_string(),
        v => v,
    }
}

/// Note coins a key will show soon (a watchlist), so the next refresh fetches them together.
pub fn want(entries: &[String], vs: &str) {
    let vs = norm_vs(vs);
    let mut guard = GECKO.lock().unwrap();
    let g = guard.get_or_insert_with(Gecko::default);
    let now = Instant::now();
    for e in entries {
        g.wanted.insert((vs.clone(), e.trim().to_lowercase()), now);
    }
}

/// One coin's quote. `entry` is a symbol ("BTC") or a CoinGecko id ("bitcoin"); `vs` a currency
/// CoinGecko knows ("usd", "eur", "btc"). `week` picks the 7-day span over 24 hours.
pub fn crypto(entry: &str, vs: &str, week: bool) -> Result<Quote> {
    let entry = entry.trim().to_lowercase();
    let vs = norm_vs(vs);
    if entry.is_empty() {
        return Err(Error::msg(
            "Enter a coin (BTC, or its CoinGecko id like bitcoin)",
        ));
    }
    let mut guard = GECKO.lock().unwrap();
    let g = guard.get_or_insert_with(Gecko::default);
    let key = (vs.clone(), entry.clone());
    let now = Instant::now();
    g.wanted.insert(key.clone(), now);
    let cached = g.coins.get(&key).cloned();
    let fresh = cached
        .as_ref()
        .is_some_and(|(t, _)| now.duration_since(*t) < CRYPTO_TTL);
    let gap = if cached.is_none() {
        GECKO_NEW_GAP
    } else {
        GECKO_GAP
    };
    let resting = g.backoff.is_some_and(|b| now < b)
        || g.tried
            .get(&vs)
            .is_some_and(|t| now.duration_since(*t) < gap);
    let mut err = None;
    if !fresh && !resting {
        if let Err(e) = gecko_fetch(g, &vs) {
            err = Some(e);
        }
    }
    match g.coins.get(&key).cloned() {
        Some((t, Some(c))) if err.is_none() || now.duration_since(t) < STALE_OK => {
            Ok(coin_quote(&c, &vs, week))
        }
        Some((_, None)) => Err(Error::msg(format!("CoinGecko has no coin \"{entry}\""))),
        _ => Err(err.unwrap_or_else(|| Error::msg("Waiting for CoinGecko…"))),
    }
}

fn coin_quote(c: &Coin, vs: &str, week: bool) -> Quote {
    let points = if week {
        c.spark.clone()
    } else {
        c.spark[c.spark.len().saturating_sub(24)..].to_vec()
    };
    Quote {
        symbol: c.symbol.clone(),
        name: c.name.clone(),
        price: c.price,
        change: if week { c.ch7d } else { c.ch24 },
        currency: vs.to_uppercase(),
        points,
        decimals: None,
        url: format!("https://www.coingecko.com/en/coins/{}", c.id),
    }
}

// ── Yahoo Finance (stocks, ETFs, indices, forex) ──

type StockCache = HashMap<(String, bool), (Instant, Quote)>;
static STOCKS: Mutex<Option<StockCache>> = Mutex::new(None);

/// Yahoo's chart reply → a quote.
pub fn parse_chart(symbol: &str, v: &Value) -> Result<Quote> {
    let r = &v["chart"]["result"][0];
    if r.is_null() {
        let why = v["chart"]["error"]["description"]
            .as_str()
            .unwrap_or("no data");
        return Err(Error::msg(format!("No quote for \"{symbol}\" ({why}). Use Yahoo symbols: AAPL, ^GSPC, EURUSD=X, RELIANCE.NS")));
    }
    let m = &r["meta"];
    let price = m["regularMarketPrice"]
        .as_f64()
        .ok_or_else(|| Error::msg(format!("No price for \"{symbol}\"")))?;
    let base = m["chartPreviousClose"]
        .as_f64()
        .or(m["previousClose"].as_f64())
        .filter(|b| *b != 0.0);
    let mut points: Vec<f64> = r["indicators"]["quote"][0]["close"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    if points.last() != Some(&price) {
        points.push(price);
    }
    let sym = m["symbol"].as_str().unwrap_or(symbol).to_string();
    let name = m["shortName"]
        .as_str()
        .or(m["longName"].as_str())
        .unwrap_or(&sym)
        .to_string();
    // Indices read better by name ("S&P 500") than by symbol ("^GSPC").
    let shown = if sym.starts_with('^') {
        name.clone()
    } else {
        sym.trim_end_matches("=X").to_string()
    };
    Ok(Quote {
        symbol: shown,
        name,
        price,
        change: base.map(|b| (price - b) / b * 100.0),
        currency: m["currency"].as_str().unwrap_or("").to_uppercase(),
        points,
        decimals: m["priceHint"].as_u64().map(|d| d.min(8) as usize),
        url: format!("https://finance.yahoo.com/quote/{}", enc(&sym)),
    })
}

fn yahoo(symbol: &str, week: bool) -> Result<Quote> {
    let (range, interval) = if week { ("5d", "30m") } else { ("1d", "5m") };
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}",
        enc(symbol)
    );
    let v: Value = match agent()
        .get(&url)
        .query("range", range)
        .query("interval", interval)
        .call()
    {
        Ok(r) => r
            .into_json()
            .map_err(|e| Error::msg(format!("Yahoo Finance sent an invalid reply: {e}")))?,
        // 404 still carries the JSON error ("symbol may be delisted").
        Err(ureq::Error::Status(404, r)) => r.into_json().unwrap_or_default(),
        Err(ureq::Error::Status(code, _)) => {
            return Err(Error::msg(format!("Yahoo Finance error {code}")))
        }
        Err(e) => return Err(Error::msg(format!("Yahoo Finance unavailable ({e})"))),
    };
    parse_chart(symbol, &v)
}

fn finnhub_key() -> String {
    knobdeck_sdk::secrets::env_or_stored("FINNHUB_API_KEY")
}

/// Finnhub's quote: price and day change only (its candles are a paid feature, so no chart).
pub fn parse_finnhub(symbol: &str, v: &Value) -> Result<Quote> {
    let price = v["c"].as_f64().unwrap_or(0.0);
    if price == 0.0 && v["t"].as_i64().unwrap_or(0) == 0 {
        return Err(Error::msg(format!("Finnhub has no quote for \"{symbol}\"")));
    }
    Ok(Quote {
        symbol: symbol.to_string(),
        name: symbol.to_string(),
        price,
        change: v["dp"].as_f64(),
        currency: String::new(),
        points: vec![],
        decimals: None,
        url: format!("https://finance.yahoo.com/quote/{}", enc(symbol)),
    })
}

fn finnhub(symbol: &str, key: &str) -> Result<Quote> {
    let v: Value = match agent()
        .get("https://finnhub.io/api/v1/quote")
        .query("symbol", symbol)
        .query("token", key)
        .call()
    {
        Ok(r) => r
            .into_json()
            .map_err(|e| Error::msg(format!("Finnhub sent an invalid reply: {e}")))?,
        Err(ureq::Error::Status(401 | 403, _)) => {
            return Err(Error::msg("Finnhub refused the API key: check it in setup"))
        }
        Err(ureq::Error::Status(429, _)) => {
            return Err(Error::msg("Finnhub's free limit was reached"))
        }
        Err(ureq::Error::Status(code, _)) => {
            return Err(Error::msg(format!("Finnhub error {code}")))
        }
        Err(e) => return Err(Error::msg(format!("Finnhub unavailable ({e})"))),
    };
    parse_finnhub(symbol, &v)
}

/// A stock, ETF, index or forex pair by its Yahoo symbol.
pub fn stock(symbol: &str, week: bool) -> Result<Quote> {
    let symbol = symbol.trim().to_uppercase();
    if symbol.is_empty() {
        return Err(Error::msg("Enter a symbol (AAPL, ^GSPC, EURUSD=X)"));
    }
    let key = (symbol.clone(), week);
    let cached = STOCKS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&key))
        .cloned();
    if let Some((t, q)) = &cached {
        if t.elapsed() < STOCK_TTL {
            return Ok(q.clone());
        }
    }
    let got = yahoo(&symbol, week).or_else(|e| {
        let k = finnhub_key();
        // An unknown symbol stays an error; only an unreachable or refusing Yahoo falls back.
        if k.is_empty() || e.message().starts_with("No quote") {
            Err(e)
        } else {
            finnhub(&symbol, &k)
        }
    });
    match got {
        Ok(q) => {
            STOCKS
                .lock()
                .unwrap()
                .get_or_insert_with(HashMap::new)
                .insert(key, (Instant::now(), q.clone()));
            Ok(q)
        }
        Err(e) => match cached {
            Some((t, q)) if t.elapsed() < STALE_OK => Ok(q),
            _ => Err(e),
        },
    }
}

/// Check a CoinGecko Demo key (setup).
#[allow(dead_code)] // used by finance-setup
pub fn check_gecko_key(key: &str) -> Result<()> {
    match agent()
        .get("https://api.coingecko.com/api/v3/ping")
        .set("x-cg-demo-api-key", key)
        .call()
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, r)) => Err(Error::msg(format!(
            "CoinGecko refused the key ({code}): {}",
            r.into_string()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        ))),
        Err(e) => Err(Error::msg(format!("CoinGecko unavailable ({e})"))),
    }
}

/// Check a Finnhub key (setup).
#[allow(dead_code)] // used by finance-setup
pub fn check_finnhub_key(key: &str) -> Result<()> {
    finnhub("AAPL", key).map(|_| ())
}

// ── Formatting ──

fn group(int: u64) -> String {
    let s = int.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "$63,491", "€1,643.84", "0.0001234 BTC", "₹2,941.10".
#[cfg(test)]
fn price(v: f64, currency: &str) -> String {
    price_with(v, currency, None)
}

/// `price`, with the source's decimals when it gives them (never fewer than 2 below 10,000).
pub fn price_with(v: f64, currency: &str, hint: Option<usize>) -> String {
    let a = v.abs();
    let decimals = if let Some(d) = hint.filter(|_| a < 10_000.0) {
        d.max(2)
    } else if a >= 10_000.0 {
        0
    } else if a >= 1.0 || a == 0.0 {
        2
    } else {
        // Four significant digits for small prices.
        ((-a.log10()).floor() as usize + 4).min(10)
    };
    let text = format!("{a:.decimals$}");
    let (int, frac) = text
        .split_once('.')
        .map_or((text.as_str(), ""), |(i, f)| (i, f));
    let num = format!(
        "{}{}{}{}",
        if v < 0.0 { "-" } else { "" },
        group(int.parse().unwrap_or(0)),
        if frac.is_empty() { "" } else { "." },
        frac
    );
    match currency {
        "USD" => format!("${num}"),
        "EUR" => format!("€{num}"),
        "GBP" => format!("£{num}"),
        "JPY" | "CNY" => format!("¥{num}"),
        "INR" => format!("₹{num}"),
        "KRW" => format!("₩{num}"),
        "" => num,
        c => format!("{num} {c}"),
    }
}

/// "▲1.23%", "▼0.40%", "" when unknown.
pub fn change(v: Option<f64>) -> String {
    match v {
        Some(c) if c >= 0.005 => format!("▲{c:.2}%"),
        Some(c) if c <= -0.005 => format!("▼{:.2}%", -c),
        Some(_) => "0.00%".into(),
        None => String::new(),
    }
}

pub fn trend_icon(v: Option<f64>) -> &'static str {
    match v {
        Some(c) if c >= 0.005 => "trending-up",
        Some(c) if c <= -0.005 => "trending-down",
        _ => "minus",
    }
}

/// The series as `width` block characters (bucket averages, scaled min..max).
pub fn sparkline(points: &[f64], width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if points.len() < 2 || width == 0 {
        return String::new();
    }
    let width = width.min(points.len());
    let buckets: Vec<f64> = (0..width)
        .map(|i| {
            let (a, b) = (
                i * points.len() / width,
                ((i + 1) * points.len() / width).max(i * points.len() / width + 1),
            );
            points[a..b].iter().sum::<f64>() / (b - a) as f64
        })
        .collect();
    let (lo, hi) = buckets
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    if hi - lo <= f64::EPSILON * hi.abs().max(1.0) {
        return BARS[3].to_string().repeat(width);
    }
    buckets
        .iter()
        .map(|v| BARS[(((v - lo) / (hi - lo)) * 7.0).round() as usize])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    #[test]
    fn prices() {
        assert_eq!(price(63491.0, "USD"), "$63,491");
        assert_eq!(price(1643.844, "EUR"), "€1,643.84");
        assert_eq!(price(0.000123456, "BTC"), "0.0001235 BTC");
        assert_eq!(price(0.5, ""), "0.5000");
        assert_eq!(price(1234567.8, "INR"), "₹1,234,568");
        assert_eq!(price_with(1.10234, "USD", Some(4)), "$1.1023");
        assert_eq!(price_with(352.45, "USD", Some(2)), "$352.45");
    }

    #[test]
    fn changes_and_lines() {
        assert_eq!(change(Some(1.234)), "▲1.23%");
        assert_eq!(change(Some(-0.4)), "▼0.40%");
        assert_eq!(change(Some(0.001)), "0.00%");
        assert_eq!(trend_icon(Some(-2.0)), "trending-down");
        assert_eq!(
            sparkline(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], 8),
            "▁▂▃▄▅▆▇█"
        );
        assert_eq!(sparkline(&[5.0, 5.0, 5.0], 3), "▄▄▄");
        assert_eq!(sparkline(&[1.0], 8), "");
        assert_eq!(
            sparkline(&(0..168).map(f64::from).collect::<Vec<_>>(), 10)
                .chars()
                .count(),
            10
        );
    }

    #[test]
    fn coin_lookup_rule() {
        assert!(symbol_first("btc"));
        assert!(!symbol_first("bitcoin"));
        assert!(!symbol_first("usd-coin"));
        assert_eq!(enc("^GSPC"), "%5EGSPC");
        assert_eq!(enc("EURUSD=X"), "EURUSD%3DX");
    }

    #[test]
    fn yahoo_reply() {
        let v = json!({"chart": {"result": [{"meta": {"currency": "USD", "symbol": "^GSPC", "regularMarketPrice": 7323.63, "chartPreviousClose": 7340.53, "shortName": "S&P 500"},
            "indicators": {"quote": [{"close": [7352.3, null, 7330.2]}]}}], "error": null}});
        let q = parse_chart("^GSPC", &v).unwrap();
        assert_eq!(q.symbol, "S&P 500");
        assert_eq!(q.points, vec![7352.3, 7330.2, 7323.63]);
        assert!((q.change.unwrap() + 0.2302).abs() < 0.001);
        assert_eq!(q.url, "https://finance.yahoo.com/quote/%5EGSPC");
        let missing = json!({"chart": {"result": null, "error": {"code": "Not Found", "description": "No data found, symbol may be delisted"}}});
        assert!(parse_chart("NOPE", &missing)
            .unwrap_err()
            .message()
            .starts_with("No quote"));
    }

    #[test]
    fn finnhub_reply() {
        let q = parse_finnhub("AAPL", &json!({"c": 352.45, "dp": 0.15, "t": 1795035601})).unwrap();
        assert_eq!((q.price, q.change), (352.45, Some(0.15)));
        assert!(parse_finnhub("NOPE", &json!({"c": 0, "d": null, "dp": null, "t": 0})).is_err());
    }

    /// Live: CoinGecko's real public API (no key). `cargo test -p finance -- --ignored`.
    #[test]
    #[ignore]
    fn live_coingecko() {
        let btc = crypto("BTC", "usd", false).unwrap();
        assert_eq!(btc.symbol, "BTC");
        assert!(btc.price > 0.0 && btc.points.len() >= 20, "{btc:?}");
        // Served from the cache: no second request.
        let again = crypto("btc", "usd", true).unwrap();
        assert_eq!(again.price, btc.price);
        assert!(again.points.len() > 100);
        let err = crypto("no-such-coin-zz", "usd", false);
        assert!(err.is_err());
    }

    /// Live: Yahoo Finance's chart endpoint (no key).
    #[test]
    #[ignore]
    fn live_yahoo() {
        let q = stock("aapl", false).unwrap();
        assert_eq!(q.symbol, "AAPL");
        assert!(q.price > 0.0 && q.currency == "USD", "{q:?}");
        let idx = stock("^GSPC", true).unwrap();
        assert_eq!(idx.symbol, "S&P 500");
        assert!(stock("NOPE123ZZ", false).is_err());
    }
}
