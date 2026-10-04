# Resolver transport diagnostic categories

When a scholarly registry request fails during TLS or CONNECT-proxy setup, the
shared bibliography client previously recorded only `transport error: request
failed`. The error mapping now preserves fixed categories for TLS handshake or
certificate validation, malformed certificate data, unavailable TLS transport,
CONNECT negotiation, invalid proxy configuration, HTTP protocol/request errors
and HTTPS-only request failures.

The mapping discards inner error messages and source-chain text. Those can
contain request URLs, proxy credentials, certificate/peer details or arbitrary
server data. Tests inject dummy sensitive strings and verify exact categories
and absence of their contents from both Display and Debug. Malformed PEM and
invalid HTTP-header construction exercise actual ureq error production without
network requests.

Every new category remains `BiblioError::Transport`; retry, rejection, unresolved
and acceptance behavior stays as before. HTTP status mapping, DNS, timeout and
I/O-kind diagnostics remain intact. NativeTls/Der and unknown future variants
retain the generic sanitized fallback. There is no dependency or lock change.

The existing [registry evaluation](https://github.com/benpshore/pdftextract/issues/148)
and [web/native handoff](https://github.com/benpshore/pdftextract/issues/195#issuecomment-5976310680)
remain separate evidence. The replacement executor's captured live resolver
tests failed at transport (0passed/2failed); generic historical errors cannot
be retroactively assigned a TLS/proxy cause. No live registry request was
repeated for this patch and no resolver accuracy or abstention pass is claimed.

The bounded independent review found no mapping/API/privacy blocker and checked
that the resolver retries by error variant rather than message text. Local
workspace Clippy, Rust tests, formatting, Ruff and Python validation are recorded
in the PR. Existing unavailable Swift/CMake/CTest and previously blocked audit
access are not fresh passes. This is ordinary error reporting, with no change
to TLS verification, trust roots, proxy/network configuration or credentials.
