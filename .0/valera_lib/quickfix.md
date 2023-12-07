1) The `requests` module is optimised for huge request queries
2) As such, every new query will have to be extensively configured
3) Given I don't want to do this every time, I make a decision to sacrifice some ease of configurability in favor of reusability. Meaning, we're favoring storage of all options inside `Templates` above all other ways of doing it.

make it `start_time: Option<Timestamp>, end_time: Option<Timestamp`
If either is not provided, assume the request is singular

the way the symbol, startTime, endTime are noted is constant through all requests of a provider. So they are converted with closure provided by it.

To implement query, let's:
1. add a runtime to the `Client`, initially fixing the provided params.
1. have api's `collect_trades` correctly receive it.
1. implement the same fixed runtime for now but now with taking the symbol, start_time, end_time from the request.
1. move the block of the runtime into the `Query` and now just initialize it inside the Client.
1. ONLY NOW ensure that this is general enough to be able to take in rolled queries for klines and individual requests.


- [ ] finish the `execute` on `SubQuery`. After working, take the non-repeating part out as a closure (already have `logic` field for this).

always can expect having either Option<String> or exactly `start_time` and `end_time` in every single scope they are used.

<<<<<<< HEAD
- add `do` function on the SubQuery object. Then have it take the closure. And then I can have the needed items in the closure, without doing self-referencing stuff

You can also move items into the closure with `move` keyword https://doc.rust-lang.org/book/ch16-01-threads.html

- So give &Arc<Mutex<Vec<Result<Whatever>>>> to every SubQuery, (and now this is what's called Query). And then provide SubQueries to the manager, defined on the provider. And they all can be handled the same way now, as every query is a self-contained runtime and some additional args to wrap it in rate-limit-aware matter
- SubQuery now has `percent_completed` field that has to be updated by the query's runtime. (when singular, update from 0 to 1). SubQuery has `estimated_time_left_s` also. It could be first set by the scheduler function of the provider. But mainly it is updated by the manager of the provider, which checks Option<timestamp_ms: u64>, (changed from None, when query's runtime is started), field on the SubQuery, with %completed in mind, time to time.
- SubQueries are immediately attached to the least busy Client on the Provider by .add_sub_query. Moved later if needed by the Manager.The only physical place where they are ever stored is according Vecs on Clients


=======
=======
>>>>>>> master
### Provider
- `build` function on the Provider should immediately initialize a new thread for the manager
- all schedulers should be implemented directly on the provider instance. As every scheduler function is immediately synonymous with a semantic query separation, we only now need to keep track of horizontal and vertical coords on the grid separation, and only within the said runtime. Every scheduler creates a new runtime.
Seems like we don't even need the `QueryGridId`. Do I just slap `GridPosition` on everything internally and that's it?
// notice that now id is the responsibility of the client. None of the apis take in id, as queries are never mixed internally.

<<<<<<< HEAD
=======
### SubQuery
- So give &Arc<Mutex<Vec<Result<Whatever>>>> to every SubQuery, (and now this is what's called Query). And then provide SubQueries to the manager, defined on the provider. And they all can be handled the same way now, as every query is a self-contained runtime and some additional args to wrap it in rate-limit-aware matter
- SubQuery now has `percent_completed` field that has to be updated by the query's runtime. (when singular, update from 0 to 1). SubQuery has `estimated_time_left_s` also. It could be first set by the scheduler function of the provider. But mainly it is updated by the manager of the provider, which checks Option<timestamp_ms: u64>, (changed from None, when query's runtime is started), field on the SubQuery, with %completed in mind, time to time.
- SubQueries are immediately attached to the least busy Client on the Provider by .add_sub_query. Moved later if needed by the Manager.The only physical place where they are ever stored is according Vecs on Clients

- I'm assuming SubQueries are actually `tokio::task` or `tokio::handler`. And then clients have one runtime on them, where we start them.

- Crate with macros for generating self-referential structs: https://docs.rs/rental/latest/rental/

- Dude solved with Rc and RefCell https://github.com/UberLambda/ttspico-rs/commit/5bdb506cd84bfe87ef50cd2433563f31883a3118
>>>>>>> master

<!--%s------------------------------------------------------------------------------
- [ ] implement manager on the provider
- [ ] `Client.assign_query()`
clients should be having their own threads, so just storing the query in their struct is enough.
- [ ] `Client.start_runtime()`
