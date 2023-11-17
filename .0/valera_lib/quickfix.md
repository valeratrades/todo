make it `start_time: Option<Timestamp>, end_time: Option<Timestamp`
If either is not provided, assume the request is singular

the way the symbol, startTime, endTime are noted is constant through all requests of a provider. So they are converted with closure provided by it.

To implement query, let's:
1. add a runtime to the `Client`, initially fixing the provided params.
1. have api's `collect_trades` correctly receive it.
1. implement the same fixed runtime for now but now with taking the symbol, start_time, end_time from the request.
1. move the block of the runtime into the `Query` and now just initialize it inside the Client.
1. ONLY NOW ensure that this is general enough to be able to take in rolled queries for klines and individual requests.


- [ ] Create a Query to it, and pass it to the end of the conveyor, assuming we don't have to split anything.

<!--%s------------------------------------------------------------------------------
- [ ] `Client.assign_query()`

- [ ] `Client.start_runtime()`
