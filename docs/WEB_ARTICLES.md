# Web article extraction

`web/lib/article-extract/` turns a fetched HTML page, or pasted HTML, into one clean article
record. It runs in the browser (the workspace calls it through `web/lib/clip.ts`) and in Node
tests through JSDOM. It executes no page scripts and fetches nothing by itself: the server route
`web/app/api/capture/route.ts` fetches the page, and the chain helper takes a loader callback.

## What it produces

`extractArticle(html, url, options)` returns an `Article` (`web/lib/article-extract/types.ts`):

| Field | Source, in priority order |
| --- | --- |
| `title` | `citation_title`, JSON-LD `headline`, `og:title`/`dc.title` without the site suffix; the page `h1` when the document title contains it; the document title without its site suffix; the Readability guess |
| `authors`, `byline` | every `citation_author`, JSON-LD `author`/`creator` (name, url, affiliation), `author`/`article:author`/`dc.creator` metas, `[rel=author]`/`[itemprop=author]` nodes; "Okonkwo, Mira" and "Mira Okonkwo" are merged |
| `published`, `modified` | `citation_publication_date`/`citation_date`/`citation_online_date`, `article:published_time`, JSON-LD dates, Dublin Core dates, `<time pubdate>`/`[itemprop=datePublished]`; `article:modified_time`, `dcterms.modified`, JSON-LD `dateModified` |
| `canonicalUrl` | `link[rel=canonical]`, `og:url`, JSON-LD `url`/`mainEntityOfPage` |
| `siteName`, `language`, `description` | `og:site_name`, JSON-LD publisher, `citation_publisher`; `html[lang]`, `citation_language`, `dc.language`, `og:locale`, JSON-LD `inLanguage`; `description`/`og:description` |
| `scholarly` | DOI, PMID, PMCID, arXiv id, journal, volume, issue, pages, publisher, ISSN/ISBN, PDF URL, abstract and keywords from `citation_*`, Dublin Core, PRISM, JSON-LD `ScholarlyArticle`/`NewsArticle` (`identifier`, `isPartOf` chain, `pageStart`), and `doi.org`/PubMed/PMC links; `null` when nothing scholarly is present |
| `html`, `markdown`, `text` | the selected article body, sanitised with DOMPurify. Markdown keeps ATX headings, `-`/`1.` lists, `>` quotes, fenced code, GFM tables, `![alt](src)` images followed by an italic caption, and starts with `# title`. Plain text keeps one block per line, list markers, quote markers, captions and tab-separated cells |
| `headings`, `images`, `links`, `tables` | evidence derived from the selected body only; images carry `data-image-id`, resolved lazy URLs and figure captions |
| `amp`, `paywall`, `nextPageUrl`, `pages` | AMP detection and `rel=amphtml`; paywall evidence; the same-site `rel=next` target; URLs merged into the record |
| `selection`, `truncated`, `repairs`, `warnings` | which selector produced the body, whether the input was cut, what was repaired, and every caveat |

Dates are normalised to ISO 8601 without applying the machine's time zone: an offset is kept when the
page gives one, a bare date stays a bare date, and unparseable strings are kept verbatim.

`toExtracted(article, capturedAt?)` projects the record onto the workspace's generic `Extracted`
shape; `clipHtml` in `web/lib/clip.ts` is that projection plus the capture timestamp. The
extractor itself is deterministic: the same input gives byte-identical output, which the tests check.

## Boilerplate removal

Hidden content (`hidden`, `aria-hidden`, `inert`, `display:none`, `visibility:hidden`) and
scripting containers are removed first, with inert `<noscript>` images promoted. Outside fragments,
navigation, banners, complementary regions, asides, footers and explicit chrome markers are dropped:
cookie and consent banners, ad slots, share bars, newsletter boxes, related/recommended/trending
modules, comment sections (`#comments`, `.comments`, Disqus, `schema.org/Comment`), paywall and
registration prompts, breadcrumbs and skip links. The marker list is explicit on purpose; substring
rules such as `*ad*` delete real article sections. `[role=doc-footnote]` content is always kept.

Selection then runs Mozilla Readability on a clone. If it returns nothing, a Readability-style
block scorer (`scoreBlocks`) votes for parents and grandparents of paragraphs by length and
punctuation, adjusts by class/id names, penalises link density and keeps the densest block. If that
is also inconclusive the cleaned `article`/`main` boundary or body is kept, with a warning. When
Readability drops tables, code, footnotes or the leading paragraph of a `main` region, the cleaned
boundary wins so structured content survives (the earlier review's rule).

## Special pages

- **AMP:** `<html ⚡>`/`amp` is reported; `amp-img`/`amp-anim` become `img` with their `srcset`;
  `amp-ad`, `amp-analytics`, `amp-sidebar`, `amp-consent`, notifications, sticky ads, iframes and
  embeds are removed; the canonical link points at the non-AMP page.
- **Paywalls:** evidence is collected from JSON-LD `isAccessibleForFree`/`hasPart`,
  `article:content_tier`, `amp-access`/`amp-subscriptions` gating and prompt elements. Only the
  publicly served text is kept: sections gated for subscribers (`amp-access` without `NOT`,
  `subscriptions-section="content"`), JSON-LD `articleBody` and cached/AMP variants are never used
  to fill in the article. The record says so in `paywall` and in a warning. No bypass is attempted.
- **Multi-page articles:** `nextPageUrl` is the first `link[rel=next]`, `a[rel=next]` or pagination
  link whose text is "Next", on the same site (subdomains and `www`/`m`/`amp` prefixes allowed) and
  not the current URL. `extractArticleChain(html, url, load, {maxPages, deadlineMs, signal})`
  follows it through the caller's loader, merges pages (repeated title dropped, images and headings
  renumbered, links deduplicated, pages wrapped in `<section data-article-page="n">`), stops on
  loops, repeated content, loader errors, the page limit (default 10) or the time budget (default
  20 s), and records why in `warnings`.
- **Lazy images:** `data-src`, `data-original`, `data-lazy-src`, `data-lazy`, `data-url`,
  `data-srcset`, `<picture><source>` sets (largest candidate wins), `<noscript>` fallbacks; spacer,
  blank, placeholder and tracking-pixel paths, `data:`/`blob:` URLs and 1×1/2×2 images are dropped.
- **Garbled or incomplete HTML:** NUL bytes are removed; an unterminated comment is neutralised so
  the text after it is parsed; unclosed `<script>`, `<style>`, `<textarea>`, `<title>`,
  `<noscript>`, `<template>`, `<iframe>` and CDATA sections at the end of a truncated page are
  closed; mis-nested and unclosed tags are left to the HTML parser. U+FFFD replacement characters
  and probable mojibake (`Ã©`, `â€™`) produce warnings. Repairs are listed in `repairs`.

## Limits and timeouts

`DEFAULT_LIMITS` (`web/lib/article-extract/index.ts`): 12 MiB of input characters (longer input is
cut, flagged `truncated` and repaired), 500 images, 5 000 links, 500 tables, 150 000 elements for
the scorer, 10 pages per chain. Each cap produces a warning when hit. Chains take a deadline and an
`AbortSignal`. Single-page extraction is synchronous and bounded by the input and element caps.

## Server fetching (`web/lib/source-fetch.ts`, `web/app/api/capture/route.ts`)

The capture route accepts a URL from the signed-in owner, fetches it and returns the bytes to the
browser, which runs the extractor. Safety:

- URL policy before any network use: `http`/`https` only, no credentials, default ports only, a
  dotted hostname that is not an IP literal (decimal, hex and integer forms are normalised by the
  URL parser and rejected), not `localhost`, `*.local`, `*.internal`, `*.intranet`, `*.corp`,
  `*.home`, `*.lan`, `*.test`, `*.invalid`, `*.example`, `*.onion`, `*.arpa`, `*.chatgpt.site` or
  `*.workers.dev`.
- The host is resolved through DNS-over-HTTPS (A and AAAA) and every answer must be globally
  routable: private, loopback, link-local (cloud metadata), CGNAT, documentation, benchmarking,
  multicast and reserved IPv4; unspecified, loopback, unique-local, link/site-local, multicast,
  documentation, Teredo, ORCHID and discard IPv6; and IPv4-mapped, IPv4-compatible, NAT64 and 6to4
  addresses are checked by their embedded IPv4 address.
- Redirects are followed manually, at most five, and each hop goes through the same policy and
  resolution. Cookies are never sent (`credentials: 'omit'`).
- A 15 s deadline covers the whole fetch (`withTimeout` also honours the request's own signal).
- Only text sources are forwarded (`text/html`, XHTML, RSS/Atom/RDF/XML, `text/plain`,
  `text/css`); the first bytes are sniffed for PDF/ZIP/GZIP/ELF/image signatures and NUL bytes.
- The body is read up to 16 MiB; longer responses are cut and flagged with `X-TPE-Truncated` so
  the extractor repairs the tail. `X-TPE-Source-URL`, `X-TPE-Redirects` and `X-TPE-Source-Bytes`
  report what was fetched, and the response carries `Content-Security-Policy: default-src 'none';
  sandbox` and `nosniff`.
- Known gap: the DNS check and the fetch are separate requests, so a resolver that answers
  differently between them (DNS rebinding) is not prevented on Workers, where the socket cannot
  be pinned to the checked address.

## Tests

From `web/`:

```sh
node lib/article-extract/tests/run.mjs
node lib/article-extract/tests/test-source-fetch.mjs
node scripts/test-clip.mjs
node scripts/test-web-extraction-review.mjs
```

`run.mjs` covers the news, scholarly, AMP, paywall, three-page, lazy-image and garbled fixtures
in `web/lib/article-extract/fixtures/` (all written for this repository, no copied articles), plus
determinism, limits, the block scorer and date normalisation. `test-source-fetch.mjs` stubs the
resolver and `fetch` to check the IP tables, the URL policy, redirect handling, timeouts, bounded
reading and binary sniffing. Nothing in these tests reaches the network.

## Not done

- The workspace still calls `clipHtml` per page; it does not yet request following pages through
  `/api/capture`, so `nextPageUrl` is reported in `metadata.nextPage` but not followed in the UI.
- JavaScript-rendered pages, authenticated pages and scroll-loaded content remain out of reach of a
  static fetch, as the extraction review already records.
- Timing against the recorded public fixtures of `docs/WEB_EXTRACTION_REVIEW.md` was not rerun;
  the synthetic suite above is the regression evidence for this change.
