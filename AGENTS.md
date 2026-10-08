# text-processing-engine

- `main` is protected. Work on a branch and open a PR (`gh pr create --fill`); the `ci` check must pass.
- Never commit secrets, `.env` files, databases or `.DS_Store` (see `.gitignore`; CI rejects them).
- Python: use **uv** only (`uv add`, `uv run`). Never pip.
- The version is the git tag; never edit a version field by hand.
- Every merge to `main` is released automatically (next minor version, wheel and sdist attached).
- Agents (including research/review-only agents) must follow
  [the accountability policy](docs/AGENT_ACCOUNTABILITY.md): register their identity/configuration,
  record actions and evidence, and add the responsible agent's trailers to commits.
  Ben's human commits are exempt; GitHub account names do not determine attribution.
- Do not spawn agents or expand spending without parent approval. The maintenance task
  under [#225](https://github.com/benpshore/pdftextract/issues/225) is sequential.
- Before committing, run:

```sh
uv run ruff format && uv run ruff check && uv run pytest && uv audit --preview-features audit-command
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
swift build && swift test
cmake -S . -B build && cmake --build build && ctest --test-dir build
```
