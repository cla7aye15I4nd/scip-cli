# scip-cli

`scip-cli` adds a [SCIP](https://github.com/scip-code/scip) index and its
source tree to a shared static code browser. Symbol occurrences link directly
to their definitions, including definitions in other files. Repeated runs add
repositories or replace their currently published commit in the same `web/`
application.

The output has no server-side runtime or framework. It uses hash routes that do
not require server-side rewrites, loads source files on demand, and virtualizes
source lines so very large files do not create a very large DOM.

The browser has two route types:

- `/`: repository catalog;
- `/#/<repo>/<commit>/<path>`: source reading and symbol navigation.

## Build and run

```bash
cargo build --release --locked --bin scip-cli

./target/release/scip-cli index.scip \
  --source-root /path/to/zlib \
  --repo-url https://github.com/madler/zlib.git \
  --commit e3dc0a85b7032e98380dec011bc8f2c2ee0d8fca \
  --output-dir site \
  --title zlib
```

The source root can be omitted when the SCIP metadata contains an accessible
`file://` project root, or when every document embeds its source text.

Serve the directory over HTTP during development or deployment:

```bash
python3 -m http.server --directory site 8080
```

Then open `http://localhost:8080/`. Generated source data is JSON loaded on
demand, so opening `index.html` directly with `file://` is not supported.

VS Code Live Preview and similar extensions may expose the page as
`/web/index.html`. The viewer detects that subdirectory automatically and uses
hash routes, so assets and generated data continue to resolve correctly without
special extension settings.

## CLI

```text
Usage: scip-cli [OPTIONS] --repo-url <REPO_URL> --commit <COMMIT> <INDEX.SCIP>

Options:
  -r, --source-root <SOURCE_ROOT>  Repository root containing indexed files
      --repo-url <REPO_URL>        Canonical repository URL
      --commit <COMMIT>            Indexed commit or stable revision
  -o, --output-dir <OUTPUT_DIR>    Static site output directory [default: site]
      --title <TITLE>              Browser title [default: SCIP source browser]
  -h, --help                       Print help
  -V, --version                    Print version
```

Existing projects and unrelated files are preserved. The generated layout is:

```text
site/
├── index.html
├── 404.html
├── _redirects
├── assets/
│   ├── app.js
│   └── style.css
└── generated/
    ├── catalog.json
    └── github-com-madler-zlib/
        └── e3dc0a85b7032e98380dec011bc8f2c2ee0d8fca/
            ├── manifest.json
            └── files/
                ├── 0.json
                └── ...
```

The root page lists all generated repositories and their current commits. Project and file
URLs use static-host-compatible hash routes:

```text
/#/github-com-madler-zlib/e3dc0a85b7032e98380dec011bc8f2c2ee0d8fca/
/#/github-com-madler-zlib/e3dc0a85b7032e98380dec011bc8f2c2ee0d8fca/deflate.c?line=120
```

No host-level fallback or rewrite is required. `_redirects` and `404.html` are
still emitted for compatibility with hosts that support those conventions.

## Navigation behavior

- Clicking an occurrence jumps to its first definition in the index.
- Symbols defined outside the index are identified but are not links.
- URLs include the repository slug, commit, source path, and optional
  `?line=<zero-based-line>&char=<utf16-column>` location.
- UTF-8, UTF-16, and UTF-32 SCIP position encodings are normalized to browser
  UTF-16 offsets during conversion.
