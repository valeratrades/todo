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

always can expect having either Option<String> or exactly `start_time` and `end_time` in every single scope they are used.

<<<<<<< HEAD
- add `do` function on the SubQuery object. Then have it take the closure. And then I can have the needed items in the closure, without doing self-referencing stuff.
=======
### Provider
- `build` function on the Provider should immediately initialize a new thread for the manager
- all schedulers should be implemented directly on the provider instance. As every scheduler function is immediately synonymous with a semantic query separation, we only now need to keep track of horizontal and vertical coords on the grid separation, and only within the said runtime. Every scheduler creates a new runtime.
Seems like we don't even need the `QueryGridId`. Do I just slap `GridPosition` on everything internally and that's it?
// notice that now id is the responsibility of the client. None of the apis take in id, as queries are never mixed internally.

No matter how I'm doing this, assigning function to the client would always 
>>>>>>> d0d1d79 (.)

<!--%s------------------------------------------------------------------------------
- [ ] implement submit on the provider
- [ ] `Client.assign_query()`
- [ ] `Client.start_runtime()`
