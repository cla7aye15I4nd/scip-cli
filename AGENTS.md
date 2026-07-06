# Repository guidance

## Purpose

`scip-cli` is a small, single-purpose converter. It accepts an existing SCIP index and source tree and writes a framework-free static code browser. Keep repository checkout, source builds, indexer downloads, scheduling, and deployment in GitHub Actions—not in the Rust CLI.

## Architecture

- `src/`: SCIP-to-static-site conversion only.
- `assets/`: static browser shell. Use hash routes so static hosts need no SPA rewrite.
- `.github/projects/*.yaml`: one self-contained definition per indexed project.
- `.github/workflows/code-browser.yml`: inline project planning, state restore, fragment assembly, validation, R2 publication, and Pages deployment.
- `.github/workflows/project-build.yml`: inline reusable workflow for one project.
- `.github/scripts/build-cmake-project.sh`: the only shared project build helper; workflow orchestration belongs directly in YAML.

The planner compares every upstream HEAD with the previous version-2 catalog and does not spawn a build when that commit is already indexed. Pull requests prefer their own last successful preview state, allowing projects to be added in verified batches without rebuilding earlier batches. The assembler exposes exactly one current commit per repository. Production state contains only the Pages shell, catalog, and manifests; immutable compressed source packs live in R2.

## Project definitions

Add projects only through `.github/projects/<name>.yaml`. The filename and `name` must match. Required fields are:

```yaml
name: example
repoUrl: https://github.com/owner/example.git
build: |
  exec .github/scripts/build-cmake-project.sh -DBUILD_EXAMPLES=OFF
```

The build runs with `SOURCE_DIR`, `BUILD_DIR`, and `BUILD_JOBS` exported. It must leave `compile_commands.json` at `$BUILD_DIR/compile_commands.json`. Prefer the shared CMake helper. For configure/Make projects, use `bear`. Disable tests, examples, benchmarks, documentation, and optional tools unless they generate source required by the library.

Choose small, independently buildable C or C++ repositories with meaningful attack surface: parsers, codecs, decompressors, protocol implementations, database formats, or components that otherwise process untrusted input. Do not add pure computation helpers, formatting libraries, CPU detection, checksums, or similar utilities merely because they are common. Avoid projects requiring depot_tools, large dependency syncs, proprietary SDKs, or multi-hour builds. Use canonical upstream URLs and verify Chromium usage from Chromium's `README.chromium` when making that claim.

## Static-site constraints

- Store each source file as an independently gzip-compressed JSON record inside one immutable pack per project commit. The manifest must carry exact byte offsets and lengths so the browser can issue one Range request per source file.
- Preserve atomic catalog and commit publication.
- Expose exactly one commit per project. Old content-addressed R2 objects may remain briefly for safe rollout, but must not remain in the catalog.
- Keep Pages limited to the browser shell, catalog, and manifests. Publish data packs to the `scip-cli-data` R2 bucket and serve them from `https://data.code.dataisland.org`.
- Cloudflare Pages limits remain no more than 20,000 files and no individual file over 25 MiB; R2 packs are not subject to the Pages file-size limit.
- Do not introduce a dynamic application backend, generated JavaScript data copies, vulnerability views, or build orchestration into the CLI.

## Validation

Run relevant checks before publishing:

```bash
cargo fmt --all -- --check
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
node --check assets/app.js
bash -n .github/scripts/build-cmake-project.sh
```

Also parse every project YAML, syntax-check its embedded `build` block, run ShellCheck on scripts, and run actionlint on workflows. A project addition is not complete until its real matrix job, site assembly, catalog-size check, and Chrome source-render smoke test pass on GitHub Actions.

## Product and publishing

- UI branding is `SCIP-CLI`.
- Keep README short and user-facing; do not document CI internals there.
- Publish changes through a focused PR and preserve unrelated worktree changes.
- Production is the Cloudflare Pages project `scip-cli`, with canonical custom domain `code.dataisland.org`.
- Project packs are public immutable R2 objects under `data.code.dataisland.org`; configure CORS for `https://code.dataisland.org` and `https://scip-cli.pages.dev` with the `Range` request header exposed.
- Production workflows must fail before building when Cloudflare credentials or R2 access are missing; never silently skip deployment.
