# Finance

Crypto, stock, ETF, index and forex prices on KnobDeck keys and the dial. Works on Windows, macOS
and Linux. No account needed.

## What it does

- **Ticker**: one coin or symbol per key: price, change and a sparkline over 24 hours / today or
  7 days / 5 trading days. The arrow icon follows the change. Pressing opens the coin's page on
  CoinGecko or the symbol's page on Yahoo Finance.
- **Watchlist Wheel**: up to 16 coins and symbols on the dial, each with its price and change;
  pressing a slice opens its page.
- **Watchlist Dial**: turn through the same list; the key shows the current item like a Ticker.

## Symbols

- **Coins**: a symbol (`BTC`, `ETH`, `SOL`: the coin with the largest market cap wins) or a
  CoinGecko id (`bitcoin`, `usd-coin`, shown on the coin's CoinGecko page as "API ID"). Prices in
  any currency CoinGecko quotes in (`usd`, `eur`, `inr`, `btc`…).
- **Stocks, ETFs, indices, forex**: Yahoo Finance symbols: `AAPL`, `VOO`, `^GSPC`, `^NSEI`,
  `EURUSD=X`, `RELIANCE.NS` (NSE), `VOD.L` (London). Prices are in the exchange's currency.

## Sources and limits

- **CoinGecko** public API (no key). Every coin any key shows is fetched in one batched request
  per currency, at most every two minutes. On CoinGecko's rate limit the keys keep the last
  prices and the plugin waits a minute. A free **Demo key** (CoinGecko ▸ Developer Dashboard),
  entered in setup, raises the limit for long watchlists.
- **Yahoo Finance**'s chart endpoint (no key), once a minute per symbol. It is not an official
  API, so as a fallback setup takes a free **Finnhub** key (finnhub.io): when Yahoo can't be
  reached, stock prices come from Finnhub's quote endpoint (price and day change, no chart).
  Free keyless stock sources that work today are rare: Stooq now blocks scripts, and Alpha
  Vantage, Twelve Data and Finnhub all need a key.

Keys are kept in this computer's credential store. Prices are for information only and may be
delayed.
