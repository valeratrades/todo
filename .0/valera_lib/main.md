# Thoughts
To make generalized request circle for both klines and trades aggTrades collection, have to turn the latter around and go backwards.

## Providers
Should somehow add a field for storing endpoint-specific details, like `weight` and `extract_fields`

### Centralised average rt
Let's keep a centralised average `rt` on each Provider, updating every minute.

- just add it inside the `Client::start_runtime`
- Submit `(n, total_time, timestamp)` to the `.update_rt()` on the Provider. Inside we get av, while multiplying everyone by `(120-(Utc::now().diff(entry~i~.timestamp)`
- and `.update_rt()` is called right from `Client::start_runtime`

## Schedulers
Could also store the full initial number of fields in the Id field of every query produced during splitting.
And then, after reconstructed, count how many are missing; whether it is acceptable.

## Clients
We just create a new `tokio::Runtime` for every one of the queries. And then we refer to the runtimes instead of the queries forever after.
We're not transferring any `Queries` for which a runtime has been initiated.

- goal: equalize the estimated time of completion of all queries on each client.

### Proxy
Storing as Option<String>.
During the request, if Some, add a layer that reroutes it, (or is there an option for this in `reqwest`?).

## Query
Every query is synchronous. We rely on scheduler to take care of splitting and separating where possible.

The only thing client does is starting a runtime for it, where we continuously 1) request 2) concat, update start_time 3) check if `> end_time`, delete extras if yes
  // last step is present for collecting from klines endpoints too, in the name of generalisation.

It is queries who hold the method for extracting the fields from each request
