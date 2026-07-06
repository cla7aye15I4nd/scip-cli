# scip-cli

`scip-cli` is a single-purpose converter: it reads one existing SCIP index and
its source tree, then writes a framework-free static code browser. It does not
clone repositories, build source code, invoke an indexer, download tools, or
schedule projects.

The only enabled deployment is zlib. Its checkout, CMake build, `scip-clang`
indexing, conversion, validation, and Cloudflare Pages deployment are defined
in [`.github/workflows/zlib-pages.yml`](.github/workflows/zlib-pages.yml).

## Build

```bash
cargo build --release --locked --bin scip-cli
```

## Convert a SCIP index

```bash
scip-cli index.scip \
  --source-root /path/to/checkout \
  --repo-url https://github.com/madler/zlib.git \
  --commit e3dc0a85b7032e98380dec011bc8f2c2ee0d8fca \
  --output-dir site \
  --title zlib
```

The output contains the application shell, a repository catalog, one manifest,
and lazy per-source-file JSON. Serve it over HTTP; direct `file://` access is not
supported. Navigation uses hash routes, so no server-side SPA rewrite is needed.

```text
site/
├── index.html
├── assets/
│   ├── app.js
│   └── style.css
└── generated/
    ├── catalog.json
    └── github-com-madler-zlib/<commit>/
        ├── manifest.json
        └── files/*.json
```

## zlib CI and Cloudflare Pages

The GitHub Actions workflow runs on changes to the converter, frontend, lockfile,
or workflow. Pull requests build and validate a one-day artifact. Pushes to
`main` additionally deploy when both repository secrets are configured:

- `CLOUDFLARE_ACCOUNT_ID`
- `CLOUDFLARE_API_TOKEN`

The API token only needs `Account / Cloudflare Pages / Edit` for the selected
account. Do not reuse Wrangler's broad local OAuth token in CI.

Production: <https://scip-cli-zlib.pages.dev/>

## Scope

The converter intentionally has no repository profiles or build configuration.
Adding another repository requires a separate CI workflow and is outside the
current zlib-only scope.
