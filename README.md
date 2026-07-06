# scip-cli

A small converter that turns a SCIP index and its source tree into a static code browser.

```bash
cargo build --release --locked --bin scip-cli

target/release/scip-cli index.scip \
  --source-root /path/to/source \
  --repo-url https://github.com/owner/repo.git \
  --commit "$(git -C /path/to/source rev-parse HEAD)" \
  --output-dir site \
  --title repo
```

The generated site has no server-side runtime and can be served by any static host.

<https://code.dataisland.org>
