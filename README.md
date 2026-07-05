# scip-cli

The initial CLI exposes repository profile discovery and inspection:

```bash
cargo run -p scip-cli -- list
cargo run -p scip-cli -- show harfbuzz
```

Profiles live under `configs/`. Use `--config-dir` or the
`SCIP_CLI_CONFIG_DIR` environment variable to select another directory.
