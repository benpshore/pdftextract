# Physical-layout fixtures

These are authored synthetic PDF operator programs, not private documents or
corpus downloads. `generate.py` writes eight deterministic, uncompressed PDFs
without importing any extraction/layout code. The `.txt` expectations were
written by hand and are **not** generated from the implementation or Poppler.
Their final newline is a fixture-file convention; the API adds no final newline.

- `columns.pdf`: scrambled stream order, 20-point heading, two aligned columns,
  indentation, three table cells, and deliberately empty rows. Body text uses
  Courier at 10 points (6-point character advance, 12-point grid rows).
- `special.pdf`: Type0 `ToUnicode` maps é, 中 and 😀, same-row overlapping runs
  (`AB` then `CD` in spatial order), an individual vertical run, and decoded
  source tab/newline characters. Unicode expectations are decoded evidence;
  the fixture does not embed a font program or establish visual glyph rendering.
- `rotate-{0,90,180,270}.pdf`: the same two display-frame lines using independent
  inverse text matrices and each page's `/Rotate`. All use `rotated.txt`.
- `semantic-{two,three}.pdf`: tightly spaced headings over two/three columns,
  with handwritten column-major and paragraph expectations in the corresponding
  `.txt` files. These reproduce unresolved default reading-order failures; the
  explicitly ignored checkpoint acceptance tests are expected to fail when run.

Regenerate with `uv run python tests/fixtures/layout_text/generate.py` and check
`git diff --exit-code -- tests/fixtures/layout_text/*.pdf`. Expected files must
remain independently authored. Tests open the committed bytes through the real
`LopdfBackend` and assert decoded strings before invoking the layout API.
No PR246 superscript comparison harness or outputs are used here.
