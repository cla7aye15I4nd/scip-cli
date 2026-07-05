# scip-cli

The CLI exposes repository profile discovery, inspection, and index generation:

```bash
cargo run -p scip-cli -- list
cargo run -p scip-cli -- show harfbuzz
cargo run -p scip-cli -- generate harfbuzz --output-dir scip-output
```

Profiles live under `configs/`. Use `--config-dir` or the
`SCIP_CLI_CONFIG_DIR` environment variable to select another directory.
