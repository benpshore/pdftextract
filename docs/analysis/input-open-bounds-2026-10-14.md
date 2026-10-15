# Reject a FIFO substituted between observation and open

Base: `8f7272a94833d9c06eb55b93900a44791a2e6665`.
This is a pure-Rust acquisition change, independent of legacy PDF provider
maintenance and the Python invocation/library architecture.

`read_verified` first checks path metadata, then opens the path, then checks the
opened descriptor. A regular path can become a FIFO between the first two
operations. The descriptor check cannot reject the FIFO until `File::open`
returns, and a blocking read-only open waits for a writer. The supervised CLI
has an outer process deadline; direct library callers do not inherit it.

On Linux and macOS, open the candidate with RDONLY, CLOEXEC and NONBLOCK through
the existing safe rustix API. Check the opened descriptor's type and identity
as before. Regular file symlinks remain supported. The existing immutable
snapshot checks and byte limits are unchanged; no source is written. The `fs`
feature is enabled explicitly on the existing rustix dependency; Cargo.lock is
unchanged and no parser/native PDF dependency is introduced.

This closes the FIFO-open wait, not every possible filesystem wait. Metadata,
network filesystems, ordinary file reads and device drivers can still block.
The worker boundary remains necessary for hard document deadlines. Other
platforms retain their existing File::open path; no new coverage is claimed.

## Falsification and tests

A private helper isolates the metadata/open gap. A subprocess test observes a
regular file, renames it without modifying its bytes, substitutes a FIFO, then
continues the same production acquisition path. No race timing loop or writer
is used. The test supervisor has a five-second kill/reap deadline.

Before the open change, this test failed at five seconds because the reader
waited for a writer. After the change, both direct and symlink-mediated FIFO
substitution return NotAFile. A regular-file symlink control verifies content,
hash and unchanged original bytes. The other acquisition tests continue to
exercise file growth, path replacement, file limits and malformed inputs.

The focused Linux x86-64 run passed 11 acquisition tests; the ignored probe was
also invoked successfully by its supervisor. Strict root Clippy and all 210
Python tests passed. Full Rust suite and hosted exact-head outcomes are recorded
in the PR/handoff. macOS execution has not been performed in this environment.
Test timing is a bounded regression observation, not a throughput benchmark.
