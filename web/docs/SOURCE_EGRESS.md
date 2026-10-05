# Public source egress — issue #215

Reviewed 2026-10-04 UTC against GitHub main `72890e6f9d23b1c10ee2cd9d368c7c0817319c10`
and Site source `9f7bc798ca5b3002f77e52e9bb2c7bd2774c5f0b`.
Their `lib/source-fetch.ts` files matched byte for byte. Both capture and remote
document assets use this boundary, with manual, revalidated redirects.

The DoH preflight and hostname fetch perform separate resolutions. The checked
address is **not pinned**, and a second preflight cannot eliminate that race.
Workers supplies the connection-time public-network boundary instead.

Cloudflare documents that its outbound HTTP proxy excludes internal services,
but ordinarily permits the Worker's own-zone origin. The Site's Vite/Cloudflare
configuration previously declared only `nodejs_compat`. Add
`global_fetch_strictly_public` to remove the documented origin-routing exception:
global fetch must route through the public Internet even for the Worker's zone.
The generated `dist/server/wrangler.json` must retain this flag. This is bounded
runtime hardening, **not evidence of an exploited production rebinding attack**.

No IP-URL rewrite, forged Host header, raw socket, or `resolveOverride` is used.
Workers does not support arbitrary IP pinning with those fetch mechanisms;
`resolveOverride` is zone constrained and would be ignored for arbitrary public
sources. The retained transport supports ordinary public hostname/TLS capture.
On `integration/security-hardening`, production capture and remote assets remain
disabled before DNS/HTTP by the fixed local-only capability. There is no runtime
flag, environment switch, or test override in the application that enables it.

## Regressions and limits

After a production build, run `node scripts/test-source-fetch.mjs`.
It first checks that the unmodified production gate refuses before DNS/HTTP.
A separately bundled, explicitly test-only capability stub then exercises the
retained transport; it does not qualify production re-enablement. That fixture
checks the source and generated runtime flag, public capture, relative public
redirects, private-address and private-DNS redirects, and a same-host redirect
whose mocked DoH answer changes from public to loopback. A separate real workerd
test simulates a successful public preflight followed by a hostname connection
resolving to loopback: the public-only network rejects it before a local sentinel
receives a request. An unrestricted Node fetch reaches the same sentinel as a
positive control.

The real test exercises workerd's default public-only connection policy, not a
live authoritative-DNS flip. The strictly-public flag has a production-specific
own-zone effect that local workerd cannot reproduce. Its presence is asserted
in generated deployment configuration; no production own-zone exploit is claimed.
The current source snapshot has no VPC binding in its global fetch path. A Site
publication does not independently expose the provider's entire egress proxy
configuration, so provider enforcement remains a runtime security dependency.

Do not port this module unchanged to Node, unrestricted Miniflare egress, a VPC
network binding, or a custom global outbound service. Those hosts must reject
non-public addresses at connection time (or use a verified public-only fetch
relay). DoH-only checks are insufficient on such hosts. Connection policy also
does not address prompt injection, parser execution, or local-file clobbering.

## Primary runtime sources

- https://developers.cloudflare.com/workers/reference/security-model/
- https://developers.cloudflare.com/workers/configuration/compatibility-flags/#global-fetch-strictly-public
- https://developers.cloudflare.com/workers/runtime-apis/request/#the-cf-property-requestinitcfproperties
- https://github.com/cloudflare/workerd/blob/main/src/workerd/server/workerd.capnp
- https://github.com/cloudflare/workerd/blob/main/src/workerd/io/compatibility-date.capnp
