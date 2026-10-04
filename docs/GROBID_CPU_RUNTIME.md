# Local CPU GROBID qualification

This work supplies a reproducible, finite service qualification under epic
[#182](https://github.com/benpshore/pdftextract/issues/182). It runs the actual
GROBID CRF models on CPU, then exercises the separately selected Rust `tpe grobid`
client. It is a local VM test, not production hosting or browser integration.
GROBID consumes an existing PDF text layer; it does not perform OCR on scans.

## Pinned distribution

The official [0.9.1 release](https://github.com/grobidOrg/grobid/releases/tag/0.9.1)
and [CRF Docker instructions](https://grobid.readthedocs.io/en/latest/Grobid-docker/)
identify a CPU-only distribution. CRF uses Wapiti; the larger full distribution
also carries deep-learning dependencies and can have different accuracy/resource
behavior. This qualification does not use the full image, Python/TensorFlow, CUDA,
or any GPU. The tested image is pinned by content digest, not a floating tag:

```text
grobid/grobid@sha256:223957791ac2bbe48609dcc58a689b16b60baeae13a8734ef440ae6bfb38f4cd
```

The fetched `0.9.1-crf` tag resolved to that digest. The test records its actual
linux/amd64 image ID, `/api/version` response, Temurin Java version, all 13 CRF
model-file hashes, PDFalto hash, Wapiti native-library hash, effective Docker
limits and complete configuration hash. Packaged Java is **21.0.11+10**; the
service reports **version 0.9.1, revision 0.9.1**. ARM/iOS execution is untested.
The image digest pins the native dependencies as well as the Java distribution.
This is independent of the Rust/Docling dependency matrix.

## Reproduce

Requirements: an available Docker daemon, `curl`, Python, and enough local image
storage/RAM. No daemon setup, persistent credentials, firewall changes or public
endpoint is performed by the helper. Pulling downloads code/models only.

```sh
docker pull grobid/grobid@sha256:223957791ac2bbe48609dcc58a689b16b60baeae13a8734ef440ae6bfb38f4cd
cargo build --locked --features grobid
uv run python scripts/grobid_cpu_runtime.py \
  --tpe target/debug/tpe \
  --output-dir /tmp/tpe-grobid-cpu-new-run
```

The output directory must not already exist. The helper never accepts a user
document or a remote endpoint: it generates its own deterministic, two-page
scholarly fixture. The optional `--tpe` argument runs the actual supervised Rust
client after raw HTTP qualification; omitting it qualifies only the service API.
There is one service, one processing slot, and one client request at a time.
All work occurs in CLI/child processes, not a UI thread.

The service uses a newly allocated **127.0.0.1-only** host port. The admin port
is not published. It has two CPU cores of quota, a 4 GiB container-memory bound
with no extra swap allowance, a 2 GiB Java heap, 128 process/thread slots, and no
GPU device requests. Wapiti training threads and GROBID request concurrency are
both one; PDFalto has a 1,024 MiB limit and a 30-second deadline. Upstream
token/block safeguards remain visible in the captured configuration; the helper
does not invent a file/page admission ceiling. These settings qualify this small
fixture only, not documents of arbitrary complexity or simultaneous batches.

The runner disables model preloading to distinguish service startup from lazy
first-request model loading. Readiness is bounded, each raw HTTP transfer has a
90-second total deadline, and subprocess commands have separate finite waits.
`curl` configuration files, proxies and redirects are disabled. HTTP responses
other than 200 fail explicitly. The Rust client uses its existing supervised
60-second CLI deadline and memory-growth budget.

All three metadata-consolidation request parameters are explicitly zero. The
returned TEI also records these parameters. No user document is uploaded and no
third-party OCR/GROBID endpoint is called. Disabling consolidation is a request
contract, not a general Docker egress firewall; the host network policy is left
unchanged.

The container is removed after success, failure, Ctrl-C or SIGTERM. Cleanup
failures are retained in the report and do not erase the primary processing
error. Hard-killing the runner or losing the Docker daemon can prevent cleanup;
the report's generated container name makes an orphan identifiable. Cancellation
of a client request alone does not guarantee cancellation of server work already
accepted. This finite test removes its own whole container to stop that work.
The regular Rust client intentionally does not stop an independently operated
service.

## Evidence and acceptance

The qualification retains the synthetic original PDF, raw TEI from every raw API
request, raw Rust-client JSON, effective YAML, container log and JSON report.
Input/TEI/config/model/native hashes remain independently inspectable. Raw region
coordinates keep GROBID's page-point convention; no missing boxes are invented.

The fixture acceptance is deliberately narrow: unchanged source bytes, exact
title and two DOI strings, two parsed references, two reported page surfaces,
and coordinate evidence on both pages. Rust checks require matching source and
TEI hashes, two pages/two citations, `semantic_projection`, and a retained
warning. These criteria prove a working CPU service/client path, not complete
PDF coverage or adjudicated scholarly accuracy.

The [final fresh-service run](validation/grobid-cpu/report.json) on 2026-10-04
measured startup **2.632 s**, first request **25.370 s**, and warm raw HTTP
requests **1.078 / 1.075 s**. The actual supervised Rust client then completed
in **1.094 / 1.083 s** against that warm service. Startup is measured from
`docker start` through readiness; first/warm request times cover the whole HTTP
request and TEI receipt. Image pulling, configuration extraction and output
validation are excluded. First means a fresh service process with models not
preloaded; OS file caches were not flushed. No target-device speed is claimed.

Real model shortcomings were retained: the CRF result omitted the second abstract
sentence and treated the Introduction heading as paragraph text. It retained
27 coordinate regions, three empty coordinate attributes, two references and
both DOI strings. The report measures
those two semantic gaps explicitly. A `passed` runtime qualification must not
be interpreted as complete extraction.

Peak cgroup memory was **3,806,969,856 bytes** under the enforced 4 GiB bound.
This is the whole container's cgroup memory peak, including charged file cache;
it is not process RSS. A separate after-request Docker statistics snapshot is
also retained. Image pulling and container layers require additional disk space.

The live Rust request also exposed a contract case absent from the earlier mock
fixtures: GROBID emits `coords=""` on three citation-author `persName` elements
when those boxes are unavailable. The original client rejected that TEI. The
[exact service TEI](validation/grobid-cpu/rust-options.tei.xml),
[failure report](validation/grobid-cpu/rust-client-before-fix.json), and
[stderr](validation/grobid-cpu/rust-client-before-fix.stderr.txt) are retained.
Empty coordinate attributes are absent geometry, not zero-sized rectangles;
they must remain inspectable in raw evidence and carry a warning. Nonempty
malformed coordinates still need explicit rejection. The helper now requests
the same coordinate element types and generated IDs as the Rust client and
reports present regions separately from empty attributes.

The corrected Rust client passed against that same real service, retaining the
source/TEI hashes, two pages, two citations, `semantic_projection`, and separate
warnings for unavailable coordinates and incomplete semantic coverage. The
[live result](validation/grobid-cpu/rust-client-fixed-live.json) and
[confirmation summary](validation/grobid-cpu/rust-client-fixed-live-summary.json)
record the actual response and binary identity. The fix treats only an entirely
empty coordinate value as absent geometry; malformed nonempty tuples still fail.

`tests/test_grobid_cpu_runtime.py` checks the generated source identity and
consolidation payload, enforces a deadline against a deliberately slow loopback
server, rejects redirects even with a redirect-enabling curl configuration, and
preserves failure evidence when cleanup also fails. These isolated contracts do
not replace the actual model run. A live SIGTERM experiment during first-request
processing also returned an interruption report and removed the actual container;
the original PDF hash was unchanged. See the
[final cancellation evidence](validation/grobid-cpu/cancellation.json).

Production service lifecycle, realistic scholarly accuracy evaluation, queue
admission/backpressure and cancellation of individual accepted jobs remain open.
No account hosting capacity, browser-to-native deployment, iPhone execution or
GPU/Apple Neural Engine behavior is established here. The raw original and TEI
must remain available when adding future structured projections.
