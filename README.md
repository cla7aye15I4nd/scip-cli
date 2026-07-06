# scip-cli

`scip-cli` combines SCIP index generation and static HTML publishing. It can
clone a named repository, execute its YAML-defined setup/build workflow, write
a SCIP index, and publish indexes to a shared code browser. Profiles are
included for HarfBuzz, V8, libtiff, PHP, CPython, and FFmpeg. Every included
profile has completed an end-to-end SCIP generation run.

## Build

```bash
cargo build --release --bin scip-cli
```

The binary is written to `target/release/scip-cli`.

## Use

```bash
# Discover configured repositories.
cargo run --bin scip-cli -- list

# Inspect the effective YAML profile.
cargo run --bin scip-cli -- show harfbuzz

# Clone, build, and write scip-output/harfbuzz.scip.
cargo run --bin scip-cli -- generate harfbuzz --output-dir scip-output

# Generate V8 with more indexing workers on a high-memory machine.
cargo run --bin scip-cli -- generate v8 --output-dir scip-output --index-jobs 8

# Generate another built-in project.
cargo run --bin scip-cli -- generate libtiff --output-dir scip-output

# Preview every action without cloning or writing files.
cargo run --bin scip-cli -- generate v8 --output-dir scip-output --dry-run
```

Generate every configured project and publish its latest commit into `web/`:

```bash
cargo gen
```

By default, checkouts are retained in `.scip-cli/work` and downloaded tools in
`.scip-cli/tools`, so subsequent runs reuse them. Change these locations with
`--work-dir` and `--tools-dir`. Use `--skip-build` to rerun only the indexer
against an existing compilation database.

On subsequent runs, repositories without a configured `repository.revision` are
updated with `git pull --ff-only` before their build starts. Profiles with an explicit
revision remain pinned and are fetched at that revision.

The CLI uses `scip-clang` from `PATH`, a path passed with `--scip-clang`, or an
automatically downloaded pinned release on x86-64 Linux.

## Included projects

| Profile | Build workflow | Output |
| --- | --- | --- |
| `harfbuzz` | inherited CMake/Ninja | `harfbuzz.scip` |
| `libtiff` | inherited CMake/Ninja | `libtiff.scip` |
| `php` | inherited configure/Make with `compiledb` | `php.scip` |
| `python` | configure/Make captured with `bear` | `python.scip` |
| `ffmpeg` | inherited configure/Make with `compiledb` | `ffmpeg.scip` |
| `v8` | custom `depot_tools`/GN/Ninja workflow | `v8.scip` |

V8 requires substantial disk, memory, and build time. Start with `--dry-run`,
keep the default four index workers on machines with less than 32 GiB RAM, and
increase `--index-jobs` only when the machine has roughly 2 GiB available per
worker.

Project build dependencies are still system dependencies. For example, PHP
requires Autoconf, Bison, and re2c, while CPython requires `bear`. The CLI
installs its own `scip-clang`, `compiledb`, and profile-declared Git tools, but
it does not run a system package manager or use elevated privileges.

## YAML profiles

Profiles are loaded from `configs/`. Select another directory with the global
`--config-dir` option or the `SCIP_CLI_CONFIG_DIR` environment variable.

### Inheritance

Use `extends` to inherit a reusable base. Paths are relative to the child YAML
file. Mappings are merged recursively, scalar values are overridden, and lists
such as `commands` are replaced as a whole when the child supplies them.

The included CMake children are intentionally short:

```yaml
extends: bases/cmake-ninja.yaml

name: libtiff
description: TIFF image library and command-line tools

repository:
  url: https://gitlab.com/libtiff/libtiff.git
  directory: libtiff
  depth: 1

variables:
  cmake_args: >-
    -DCMAKE_BUILD_TYPE=Release
    -Dtiff-tests=OFF
```

The base contributes configure/build commands, the compilation-database path,
and output naming. The child supplies only repository metadata and overrides.
Base documents live under `configs/bases/`, so they are not presented as
directly runnable profiles by `scip-cli list`. Bases may themselves extend
other bases; inheritance cycles are rejected.

### Complete profile schema

```yaml
version: 1
name: example
description: Example CMake project

repository:
  url: https://github.com/example/project.git
  directory: project
  depth: 1
  # revision: v1.2.3

# Custom values can reference built-in or other custom variables.
variables:
  build_dir: build
  generated_dir: "{repo_dir}/{build_dir}/generated"

# Optional helper repositories cloned under {tools_dir}.
tools:
  - name: helper
    repository: https://github.com/example/helper.git
    destination: helper
    depth: 1

# Optional directories prepended to PATH for all commands.
path_prepend:
  - "{tools_dir}/helper/bin"

# Arbitrary trusted shell commands, executed in order.
commands:
  - name: Configure
    cwd: "{repo_dir}"
    env:
      CC: clang
    run: cmake -S . -B build -DCMAKE_EXPORT_COMPILE_COMMANDS=ON
  - name: Build
    cwd: "{repo_dir}"
    run: cmake --build build --parallel {jobs}

index:
  compilation_database: "{repo_dir}/build/compile_commands.json"
  output: "{output_dir}/{name}.scip"
  arguments:
    - --no-progress-report
```

Available template variables are:

- `{name}` and `{repo}`: profile name
- `{repo_dir}`: repository checkout
- `{work_dir}`: persistent checkout workspace
- `{tools_dir}`: persistent tool workspace
- `{output_dir}`: requested output directory
- `{jobs}`: build concurrency
- `{index_jobs}`: SCIP concurrency
- `{scip_clang}`: selected indexer path

Any key under the profile's `variables` mapping is also available as a
template variable. Expansion is recursive, while unknown variables and cycles
are errors. Built-in variable names are reserved and cannot be overridden.

The two included reusable bases are:

- `bases/cmake-ninja.yaml`: CMake configuration, Ninja build, and native CMake
  compilation database generation. Override `source_dir`, `build_dir`,
  `cmake_args`, or `build_args`.
- `bases/configure-make.yaml`: out-of-tree configure/Make build and automatic
  compilation database capture using an isolated `compiledb` virtual
  environment. Override `bootstrap_command`, `configure_script`,
  `configure_args`, `make_args`, `build_dir`, or `compiledb_version`.

To replace inherited commands for an unusual project, define a new `commands`
list in the child. See `configs/v8.yaml` for a fully custom workflow.

Profile commands are trusted shell code. Unknown YAML fields, unsafe checkout
paths, unsupported schema versions, missing compilation databases, command
failures, and suspiciously small generated indexes are treated as errors.

The V8 profile also demonstrates project-specific handling: it bootstraps
`depot_tools`, runs `gclient`, uses GN, builds generated sources, and rewrites
Clang 23-only flags in a dedicated indexing compilation database for the
currently released Clang 21-based `scip-clang`.
