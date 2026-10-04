# Local-only hardening integration

The draft `integration/security-hardening` combines #222's retained public-only
Workers transport configuration with a focused adaptation of #229's network-off
boundary on current main. Source PRs and exact archived heads remain separate.
No deployment or main promotion is authorized.

URL capture, its browser client, and remote document assets fail before DNS or
HTTP. The composer preserves a disabled URL draft and explains how to retain a
URL as text. Its remote capture tool is not registered. Local files, folders,
pasted text, local extraction, authenticated storage, reading and downloads
remain available. External images/styles/frames are omitted from local HTML;
caption text, source links, canonical/feed URLs and scholarly meta tags remain
inert evidence. Already owned stored images remain eligible for reading.
The current-main web tree has no scholarly service/resolver; the old #228 bridge,
replay, metadata resolver and UI are deliberately absent.

`remoteExtractionEnabled` and `metadataResolutionEnabled` are fixed false
constants, not permissions inferred from uploads or source URLs. No production
setting enables the transport. Re-enabling requires a separate reviewed change.
This is not a host-wide network sandbox: local browser assets and authenticated
same-origin D1/R2 operations remain necessary, and parsers are not certified safe.

`test-network-capabilities.mjs` tests the unmodified application boundary.
After building, `test-source-fetch.mjs` also checks that boundary, then creates a
separate test bundle with a virtual capability stub to exercise retained
transport behavior. Only that fixture bypasses the hold; no application guard
is changed. Generated public-only Workers flags and an isolated local workerd
connection test remain covered. That test is neither live DNS rebinding nor a
production own-zone test. Provider routing remains an external dependency.

From the repository root, the optional actual Chromium harness is:

```sh
PLAYWRIGHT_MODULE=/absolute/path/to/installed/playwright/index.js \
CHROMIUM_EXECUTABLE=/absolute/path/to/installed/chromium \
node scripts/test-web-hardening-browser.mjs
```

It creates disposable local storage and independently authored fixtures, uses
local sign-in, and drives actual application routes without API interception.
It does not install a browser, use production credentials, or deploy. Captured
reports and native/web command evidence are in the root stewardship integration
ledger. Formal security preflight resources remain unavailable; these engineering
checks do not clear #215 or establish parser safety, deployed identity, own-zone
routing, native accuracy, mobile compatibility or large-batch capacity.
