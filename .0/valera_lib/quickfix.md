Let's 1) rename requests 2) standardize requests for multiple symbols and from:to timestamps, both of which we assume are always present
And then tile request-adapters to this interface.
? Why not put this into enums, actually? So both Symbol and Vec<Symbol> are valid

#### Binance
historicalTrades takes `fromId`
aggTrades takes `fromId` + `start&end`
fundingRate takes `start&end`
klines takes `start&end`
openInterestHist takes `start&end`
longShort*Ration takes `start&end`
takerBuySellVol takes `start&end`
basis takes `start&end`
#### Bybit
/v5/market/kline `start&end`
/v5/market/funding/history `start&end`
// practically same as Binance
#### CMC
https://pro-api.coinmarketcap.com/v2/cryptocurrency/price-performance-stats/latest takes `limit` only!

## Solution
make it `start_time: Option<Timestamp>, end_time: Option<Timestamp`
If either is not provided, assume the request is singular

//NB: currently considering the outfacing api
Can have `Enum{CoinAsString: String, CoinsAsStrings: Vec<Strin>, CoinAsSymbol: Symbol, CoinsAsSymbols: Vec<Symbol>}` for symbols
In any scenario, all operations require `Vec<Symbol>`, so every single field of the enum will be converted into it.

params are always strings in the end, so can we just request `Vec<String>` for everything but `symbols`, `start_time`, `end_time`

- [ ] User-facing api: fix params

the way the symbol, startTime, endTime are noted is constant through all requests of a provider. So they are converted with closure provided by it.

- [ ] Db extraction into matching tuple, then unpack it directly into the according scheduler
- [ ] Create a Query to it, and pass it to the end of the conveyor, assuming we don't have to split anything.

<!--%s------------------------------------------------------------------------------

- [ ] query should take in a closure for extracting the fields

- [ ] turn around trades request  // should go from the youngest timestamp available

- [ ] `Client.assign_query()`

- [ ] `Client.start_runtime()`
