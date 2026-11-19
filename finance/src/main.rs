//! Finance plugin for KnobDeck: crypto (CoinGecko, no account) and stocks, ETFs, indices and forex
//! (Yahoo Finance, no account; Finnhub with the user's key as the fallback). Ticker keys with a
//! change and a sparkline, a watchlist wheel and a watchlist dial.

mod market;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, load_env, Action, Plugin, Result, Value, Visual};
use market::Quote;
use std::collections::HashMap;
use std::sync::Mutex;

const MAX_ITEMS: usize = 16;

/// Where each Watchlist Dial key was last turned to (instance id → item index).
static DIAL: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

fn dial_pos(instance: &str) -> usize {
    DIAL.lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(instance).copied())
        .unwrap_or(0)
}

/// One watched thing: a coin or a Yahoo symbol.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    Coin(String),
    Stock(String),
}

impl Item {
    fn id(&self) -> String {
        match self {
            Item::Coin(c) => format!("c:{c}"),
            Item::Stock(s) => format!("s:{s}"),
        }
    }
    fn parse_id(id: &str) -> Option<Item> {
        match id.split_once(':')? {
            ("c", c) => Some(Item::Coin(c.to_string())),
            ("s", s) => Some(Item::Stock(s.to_string())),
            _ => None,
        }
    }
    fn quote(&self, vs: &str, week: bool) -> Result<Quote> {
        match self {
            Item::Coin(c) => market::crypto(c, vs, week),
            Item::Stock(s) => market::stock(s, week),
        }
    }
    fn name(&self) -> String {
        match self {
            Item::Coin(c) | Item::Stock(c) => c.to_uppercase(),
        }
    }
}

fn list(text: &str) -> Vec<String> {
    text.split([',', '\n', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// The watchlist an action's inputs name: coins first, then stocks.
fn watchlist(inputs: &Value) -> Vec<Item> {
    let s = |k: &str| inputs[k].as_str().unwrap_or("").to_string();
    let mut items: Vec<Item> = list(&s("coins"))
        .into_iter()
        .map(|c| Item::Coin(c.to_lowercase()))
        .collect();
    items.extend(
        list(&s("stocks"))
            .into_iter()
            .map(|c| Item::Stock(c.to_uppercase())),
    );
    let mut seen = Vec::new();
    items.retain(|i| {
        !seen.contains(i) && {
            seen.push(i.clone());
            true
        }
    });
    items.truncate(MAX_ITEMS);
    items
}

/// Register a watchlist's coins so one CoinGecko call fetches them all.
fn want_all(items: &[Item], vs: &str) {
    let coins: Vec<String> = items
        .iter()
        .filter_map(|i| {
            if let Item::Coin(c) = i {
                Some(c.clone())
            } else {
                None
            }
        })
        .collect();
    market::want(&coins, vs);
}

fn currency(inputs: &Value) -> String {
    inputs["currency"]
        .as_str()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or("usd")
        .to_lowercase()
}

fn week(inputs: &Value) -> bool {
    inputs["span"] == "week"
}

/// The ticker's item: one coin or one symbol.
fn ticker_item(inputs: &Value) -> Item {
    let sym = inputs["symbol"].as_str().unwrap_or("").trim().to_string();
    if inputs["market"] == "stock" {
        Item::Stock(sym.to_uppercase())
    } else {
        Item::Coin(sym.to_lowercase())
    }
}

/// "BTC ▼1.69%\n$63,491\n▂▃▅▇▆▄▃▂▁▂".
fn tile(q: &Quote) -> Visual {
    let mut label = format!(
        "{} {}\n{}",
        q.symbol,
        market::change(q.change),
        market::price_with(q.price, &q.currency, q.decimals)
    );
    let line = market::sparkline(&q.points, 10);
    if !line.is_empty() {
        label.push('\n');
        label.push_str(&line);
    }
    Visual::new()
        .label(label.trim().to_string())
        .icon(market::trend_icon(q.change))
}

/// A wheel slice: "BTC $63,491 ▼1.69%".
fn slice_label(q: &Quote) -> String {
    format!(
        "{} {} {}",
        q.symbol,
        market::price_with(q.price, &q.currency, q.decimals),
        market::change(q.change)
    )
    .trim()
    .to_string()
}

/// A problem (unknown symbol, no network, rate limit) shows on the key itself.
fn on_key(item: &str, r: Result<Visual>) -> Result<Visual> {
    Ok(r.unwrap_or_else(|e| {
        Visual::new()
            .label(format!("{item}\n{}", e.message()).trim().to_string())
            .icon("circle-alert")
    }))
}

fn span_input() -> Value {
    input::select(
        "span",
        "Chart and change over",
        &[
            ("day", "24 hours / today"),
            ("week", "7 days / 5 trading days"),
        ],
    )
    .default_value("day")
}

fn currency_input() -> Value {
    input::text("currency", "Currency for coins").placeholder("usd").default_value("usd").with("help", "Any currency CoinGecko quotes in: usd, eur, gbp, inr, jpy, btc… Stocks show their exchange's currency.")
}

fn watch_inputs() -> Vec<Value> {
    vec![
        input::text("coins", "Coins")
            .placeholder("BTC, ETH, SOL")
            .with(
                "help",
                "Symbols or CoinGecko ids (bitcoin, usd-coin), comma separated.",
            ),
        input::text("stocks", "Stocks, ETFs, indices, forex")
            .placeholder("AAPL, VOO, ^GSPC, EURUSD=X")
            .with(
                "help",
                "Yahoo Finance symbols, comma separated: RELIANCE.NS for NSE, VOD.L for London.",
            ),
        currency_input(),
        span_input(),
    ]
}

fn open(q: &Quote) -> Result {
    knobdeck_sdk::open_url(&q.url)
}

fn main() {
    load_env();
    Plugin::new(&format!("finance-{}", std::env::consts::OS), "Finance")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Crypto, stock, ETF, index and forex prices with change and sparkline: ticker keys, a watchlist wheel and a watchlist dial. No account needed.")
        .icon("app-icon.svg")
        .setup("finance-setup", None)
        .setup_inputs(vec![
            input::text("coingecko_key", "CoinGecko Demo API key (optional)").with("type", "password").with("help", "Not needed: prices work without a key. A free Demo key (coingecko.com ▸ Developer Dashboard) raises the rate limit for long watchlists. Kept in this computer's credential store."),
            input::text("finnhub_key", "Finnhub API key (optional)").with("type", "password").with("help", "Not needed: stock prices come from Yahoo Finance without a key. A free Finnhub key (finnhub.io) is used for stock prices whenever Yahoo can't be reached (no chart then). Kept in this computer's credential store."),
        ])
        .action(
            Action::button("ticker", "Ticker")
                .icon("chart-line")
                .inputs(vec![
                    input::select("market", "Market", &[("crypto", "Crypto (CoinGecko)"), ("stock", "Stock, ETF, index or forex (Yahoo Finance)")]).default_value("crypto"),
                    input::text("symbol", "Symbol").placeholder("BTC  ·  AAPL  ·  ^GSPC  ·  EURUSD=X").with("help", "Coins: a symbol (BTC) or CoinGecko id (bitcoin). Others: the Yahoo Finance symbol."),
                    currency_input().with("showIf", json!({"key": "market", "equals": "crypto"})),
                    span_input(),
                ])
                .live(market::STOCK_TTL, |ic| {
                    let item = ticker_item(&ic.inputs);
                    on_key(&item.name(), item.quote(&currency(&ic.inputs), week(&ic.inputs)).map(|q| tile(&q)))
                })
                // Pressing opens the coin's / symbol's page.
                .on_press(|ctx| open(&ticker_item(&ctx.inputs).quote(&currency(&ctx.inputs), week(&ctx.inputs))?)),
        )
        .action(
            Action::wheel("watchlist", "Watchlist Wheel")
                .icon("list")
                .inputs(watch_inputs())
                .on_open(|o| {
                    let items = watchlist(&o.inputs);
                    if items.is_empty() {
                        return o.fail("Add coins or symbols to the watchlist");
                    }
                    let (vs, wk) = (currency(&o.inputs), week(&o.inputs));
                    want_all(&items, &vs);
                    Ok(items
                        .iter()
                        .map(|it| match it.quote(&vs, wk) {
                            Ok(q) => json!({"id": it.id(), "label": slice_label(&q), "icon": market::trend_icon(q.change)}),
                            Err(e) => json!({"id": it.id(), "label": format!("{} · {}", it.name(), e.message()), "icon": "circle-alert"}),
                        })
                        .collect())
                })
                .on_child_press(|ctx| {
                    let Some(item) = ctx.child_id.as_deref().and_then(Item::parse_id) else { return Ok(()) };
                    open(&item.quote(&currency(&ctx.inputs), week(&ctx.inputs))?)
                }),
        )
        .action(
            Action::slider("watchlist-dial", "Watchlist Dial", 0.0, (MAX_ITEMS - 1) as f64)
                .icon("list")
                .step(1.0)
                .inputs(watch_inputs())
                .initial(0)
                .live(market::STOCK_TTL, |ic| {
                    let items = watchlist(&ic.inputs);
                    let Some(item) = items.first() else { return Ok(Visual::new().label("Add coins or symbols")) };
                    want_all(&items, &currency(&ic.inputs));
                    // The live refresh shows the item the dial was last turned to.
                    let i = dial_pos(&ic.instance_id).min(items.len() - 1);
                    let item = items.get(i).unwrap_or(item);
                    on_key(&item.name(), item.quote(&currency(&ic.inputs), week(&ic.inputs)).map(|q| tile(&q).state(i)))
                })
                .on_adjust(|ctx| {
                    let items = watchlist(&ctx.inputs);
                    if items.is_empty() {
                        return ctx.fail("Add coins or symbols to the watchlist");
                    }
                    want_all(&items, &currency(&ctx.inputs));
                    let now = dial_pos(&ctx.instance_id) as f64;
                    let i = match ctx.value {
                        Some(v) => v,
                        None => now + ctx.delta.unwrap_or(0.0).signum(),
                    }
                    .round()
                    .clamp(0.0, (items.len() - 1) as f64) as usize;
                    DIAL.lock().unwrap().get_or_insert_with(HashMap::new).insert(ctx.instance_id.clone(), i);
                    let item = &items[i];
                    let v = item.quote(&currency(&ctx.inputs), week(&ctx.inputs)).map(|q| tile(&q)).unwrap_or_else(|e| Visual::new().label(format!("{}\n{}", item.name(), e.message())).icon("circle-alert"));
                    ctx.set_visual(v.state(i));
                    Ok(())
                })
                // Pressing the dial opens the shown item's page.
                .on_press(|ctx| {
                    let items = watchlist(&ctx.inputs);
                    let i = dial_pos(&ctx.instance_id);
                    let Some(item) = items.get(i).or(items.first()) else { return ctx.fail("Add coins or symbols to the watchlist") };
                    open(&item.quote(&currency(&ctx.inputs), week(&ctx.inputs))?)
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchlists() {
        let items = watchlist(&json!({"coins": "BTC, eth,BTC", "stocks": "aapl ^gspc"}));
        assert_eq!(
            items,
            vec![
                Item::Coin("btc".into()),
                Item::Coin("eth".into()),
                Item::Stock("AAPL".into()),
                Item::Stock("^GSPC".into())
            ]
        );
        assert_eq!(
            Item::parse_id(&Item::Stock("EURUSD=X".into()).id()),
            Some(Item::Stock("EURUSD=X".into()))
        );
        assert_eq!(Item::parse_id("x:1"), None);
        assert_eq!(
            ticker_item(&json!({"market": "stock", "symbol": " voo "})),
            Item::Stock("VOO".into())
        );
        assert_eq!(currency(&json!({})), "usd");
    }

    #[test]
    fn tiles() {
        let q = Quote {
            symbol: "BTC".into(),
            price: 63491.0,
            change: Some(-1.69),
            currency: "USD".into(),
            points: vec![1.0, 3.0, 2.0],
            ..Default::default()
        };
        let v = tile(&q);
        assert_eq!(v.label.as_deref(), Some("BTC ▼1.69%\n$63,491\n▁█▅"));
        assert_eq!(v.icon.as_deref(), Some("trending-down"));
        assert_eq!(slice_label(&q), "BTC $63,491 ▼1.69%");
    }
}
