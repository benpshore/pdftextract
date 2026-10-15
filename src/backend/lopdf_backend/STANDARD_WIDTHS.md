# Standard simple-font advances

`standard_widths.rs` transcribes the eight proportional Latin standard font
tables from Mozilla PDF.js `src/core/metrics.js`, commit
`519c1ba8ea02023e005dfc68c9442db1fed3fc01` (Apache-2.0; adjacent license file).
Source SHA-256: `7958e89725c43a152222d5cb6b3b6fcaa6d7995bd9bd6fe18e26d726f56a2a0c`.
Each table contains the source glyph-name/width pairs sorted by glyph name;
no font programs or PDF.js executable code are included. Courier variants
have the standard fixed 600-unit advance.

The old fallback advanced every missing-width glyph by 500 units. That is
incorrect even for ordinary `i` and `W`, and displaced later strings in a `TJ`
array. Widths now follow the rendering encoding, including Differences, not
ToUnicode (which describes extraction text). Explicit PDF widths still win.
Embedded fonts, Type3 and unrecognized names do not receive standard metrics.
The narrow Arial/TimesNewRoman aliases are substitutions and always produce
`extraction_incomplete` when used; an unknown glyph with no MissingWidth also
does. An unused font does not change status. Symbol/ZapfDingbats fallback
metrics and arbitrary embedded-program advance recovery are not implemented.

Numeric expectations in the existing Helvetica geometry tests now use its
published advances instead of the incorrect 500-unit assumption. Encoding
tests for deliberately widthless non-standard fonts retain their exact text
expectations and now assert the additional geometry uncertainty.
