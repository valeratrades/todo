# Testing

## Integration

Integration tests should focus on **data in → data out**. The goal is to verify that given some initial state, the system produces the expected final state.

Avoid testing exact stdout/stderr patterns or execution behavior. Implementation details like log messages, progress output, or internal state transitions are not what we want to verify. Good tests remain valid if the underlying code is replaced by a sufficiently smart neural net that produces correct results.

What to test:
- File contents before and after an operation
- Final state of data structures

What to avoid:
- Exact stdout/stderr messages
- Internal execution flow
- Log output patterns
- Timing or ordering of operations

If you need to verify that a specific code path was taken, use tracing and check that the relevant function was called (via `TraceLog`). But prefer verifying outputs over verifying execution paths when possible.
