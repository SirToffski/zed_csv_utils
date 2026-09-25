# Rainbow CSV with Align (Zed)

Combines [zed-rainbow-csv](https://github.com/Kalmaegi/zed-rainbow-csv) syntax
highlighting with [vscode_rainbow_csv](https://github.com/mechatroner/vscode_rainbow_csv)
Align/Shrink, via a small LSP server. Zed extensions cannot edit buffers
directly, so both modes go through LSP:

- **Whitespace Align / Shrink** (destructive): explicit `source` code actions
  ("Align CSV columns (spaces)", "Shrink CSV columns (trim spaces)"). Edits
  are computed on demand via `codeAction/resolve`. Align is deliberately *not*
  a formatter: padding changes cell values for downstream consumers, so it
  never runs on save or on `editor: format` — same as VS Code, where Align is
  an explicit command.
- **Virtual Align** (non-destructive): `textDocument/inlayHint` space pads,
  served from a per-version cache for the visible range only.

Documents with unbalanced quotes (including multiline quoted records) are
never reflowed: Align/Shrink return no edits for them.

## Installation

Install **Rainbow CSV with Align** from Zed's extension page. On first use the
extension downloads the `csv-lsp` language server for your platform from this
repository's GitHub releases and keeps it up to date. Supported: Linux, macOS
and Windows on x86_64 and aarch64.

To use your own build instead, either put `csv-lsp` on your `PATH`, or point
Zed at it in `settings.json`:

```json
{
  "lsp": {
    "csv-lsp": {
      "binary": { "path": "/absolute/path/to/csv-lsp" }
    }
  }
}
```

If you also have the original Rainbow CSV extension installed, both claim
`.csv`/`.tsv`. Pick one with Zed's `file_types` setting, e.g.
`"file_types": { "Rainbow CSV Align (,)": ["csv"] }`.

## Build

Prerequisites per OS: see `docs/setup.md`. Then:

```sh
cargo test --workspace
cargo build -p csv_lsp
cargo build --target wasm32-wasip2   # same command Zed runs on dev-install
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Binaries: `target/debug/csv-lsp(.exe)`, `target/debug/csv-align(.exe)`,
`target/wasm32-wasip2/debug/zed_csv_align.wasm`. CI (`.github/workflows/ci.yml`)
runs fmt, clippy, tests, and the wasm build on Ubuntu, Windows, and macOS.

## Development: use as a dev extension

1. Add the directory containing the built `csv-lsp` binary (e.g.
   `target/debug`) to `PATH` — the extension resolves it via
   `worktree.which("csv-lsp")`.
2. Zed command palette: `zed: extensions` -> `Install Dev Extension` ->
   pick this directory.
3. Open a `.csv` / `.tsv` file. Highlighting comes from the bundled
   tree-sitter grammars (unchanged from upstream).
4. Align/Shrink appear as code actions (`editor: toggle code actions`) and —
   once *Code Actions* is enabled in the editor toolbar settings — as a
   toolbar button. Enable inlay hints in settings for the virtual-align view.

## Use without installing the extension (CLI fallback)

```sh
cargo run --quiet -p csv_lsp --bin csv-align -- align <file> --in-place
cargo run --quiet -p csv_lsp --bin csv-align -- shrink <file> --in-place
```

Or in Zed: `task: spawn` -> `CSV: align/shrink current file` (see
`.zed/tasks.json`). The CLI refuses files with unbalanced quotes instead of
risking corruption.

## Troubleshooting

- Rebuilding `csv-lsp` while Zed runs fails with "Access is denied": Zed
  holds the binary open. Close the CSV tabs (or Zed), rebuild, reopen.
- Code-action menu empty: check the per-server logs (command palette, search
  "language server", open the `csv-lsp` logs). With `CSV_LSP_VERBOSE=1` in
  Zed's environment the server logs every code-action/resolve call; slow
  inlay requests are always logged.
- No actions offered on a file: by design when the file has unbalanced
  quotes or multiline records.

## Layout

- `extension.toml` — grammars (upstream pins) + `csv-lsp` language server for
  the 4 Rainbow languages (renamed with an `Align` suffix so they don't clash
  with the existing Rainbow CSV extension). Language `name`s must match
  `languages/*/config.toml`; `language_ids` fixes the LSP `languageId`
  explicitly instead of relying on display names.
- `languages/` — copied from upstream (`csv,tsv,ssv,psv` + `highlights.scm`),
  names adjusted as above.
- `crates/rainbow_csv_core` — dialect detection, span-tracked quoted/simple
  split (port of `csv_utils.js:split_quoted_str`, extended to tolerate
  whitespace around quoted fields), single-pass `analyze_document`
  (records, trimmed/raw widths, warning/needs flags), `align_document`,
  `shrink_document`, range-limited `pads_for_range` (port of
  `rainbow_utils.js` column stats / align / shrink / inlay computation).
  Property tests assert align idempotency, shrink stability, and virtual
  delimiter alignment.
- `crates/csv_lsp` — `csv-lsp` (LSP: codeAction + resolve, inlayHint) +
  `csv-align` CLI. Open docs are cached per sync; list-time action gating is
  O(1), edits compute on resolve.
- `src/lib.rs` + root `Cargo.toml` — the extension WASM package itself
  (`zed_extension_api 0.7.0`). Resolves `csv-lsp` from settings, then
  `PATH`, then downloads it from GitHub releases. The root
  doubles as workspace root; `default-members = ["."]` so Zed's plain
  `cargo build --target wasm32-wasip2` only builds this package.
- `samples/` — copied from upstream for manual testing, plus `vgsales.csv`
  (public Kaggle data, 16.5k rows).
- `docs/setup.md` — per-OS dependency setup.

## Limits

- Single-line records only; unbalanced-quote lines and multiline records are
  refused, never reflowed.
- No comment-prefix, dynamic separator, or whitespace-dialect support.
- Virtual alignment uses display widths and does not model editor tab stops;
  CSV fields containing tabs may be visually misaligned.
- All columns left-aligned (no decimal-point numeric alignment yet).
- Non-UTF8 files: CLI decodes lossily; LSP path is UTF-8 (JSON).

## Releasing

1. Bump `version` in `extension.toml` (and the crate versions, to keep them in
   step), commit, and push.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. `.github/workflows/release.yml` builds `csv-lsp` for all six targets and
   publishes the GitHub release only once every build has succeeded. It
   refuses tags that don't match `extension.toml`.
4. After the release is live, open the PR against `zed-industries/extensions`.

To test the builds without releasing, run the workflow manually from the
Actions tab.

## Attribution

This extension combines work from two MIT-licensed projects:

- Syntax highlighting, language configs (`languages/*/config.toml`,
  `highlights.scm`), grammar pins, and `samples/` (except `vgsales.csv`)
  come from [Kalmaegi/zed-rainbow-csv](https://github.com/Kalmaegi/zed-rainbow-csv)
  (MIT, Copyright (c) 2024 Hans). Language display names were changed to
  avoid clashing with that extension in Zed.
- The Align/Shrink/inlay algorithms in `crates/rainbow_csv_core` are a Rust
  port of the logic in [mechatroner/vscode_rainbow_csv](https://github.com/mechatroner/vscode_rainbow_csv)
  (MIT, Copyright (c) 2017 Dmitry Ignatovich) — specifically the quoted-field
  splitter (`rbql_core/rbql-js/csv_utils.js`) and the column-stat / align /
  shrink / inlay-hint computation (`rainbow_utils.js`). No VS Code source
  files are vendored; the implementation here is a clean-room port.
- At build time Zed fetches the Tree-sitter grammars from
  [coroa/rainbow-csv-tree-sitter](https://github.com/coroa/rainbow-csv-tree-sitter)
  (pinned commit in `extension.toml`). Note: that repo declares no license;
  it is referenced, not distributed, by this extension — same as upstream
  `zed-rainbow-csv` does.

Our own code (`crates/`, `src/`, `extension.toml` arrangement, tasks, docs)
is MIT licensed, see `LICENSE`.
