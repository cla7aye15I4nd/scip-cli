---
name: add-project
description: Add or update a small C or C++ repository in SCIP-CLI using its single project YAML definition, dynamic GitHub Actions matrix, SCIP indexing, static-site assembly, and validation. Use when asked to add a library, browser dependency, Chromium third-party project, or another source repository to the code browser.
---

# Add a project

Read `AGENTS.md` and the existing `.github/projects/*.yaml` files before editing.

## 1. Qualify the project

- Prefer a canonical, actively maintained upstream repository.
- Require meaningful attack surface: untrusted file/data parsing, media or font decoding, decompression, network protocols, or persistent database formats.
- Reject pure computation helpers, formatting/logging libraries, CPU detection, checksums, and similarly low-exposure utilities even when they are popular.
- Keep the project small and independently buildable on `ubuntu-24.04` with Clang.
- Reject projects that require depot_tools, a large dependency sync, proprietary SDKs, or a long monolithic build.
- Verify claims about Chromium usage against Chromium's upstream `README.chromium` or DEPS data.
- Check that the build can emit a Clang-compatible compilation database.

## 2. Add one YAML file

Create `.github/projects/<name>.yaml`; do not edit a central project list or add project-name branches to shared scripts.

```yaml
name: example
repoUrl: https://github.com/owner/example.git
build: |
  exec .github/scripts/build-cmake-project.sh \
    -DEXAMPLE_BUILD_TESTS=OFF \
    -DEXAMPLE_BUILD_TOOLS=OFF
```

The filename must equal `<name>.yaml`. The child workflow exports:

- `SOURCE_DIR`: shallow upstream checkout
- `BUILD_DIR`: build directory and required location of `compile_commands.json`
- `BUILD_JOBS`: safe parallelism value

Use `.github/scripts/build-cmake-project.sh` for ordinary CMake projects. Override `SOURCE_DIR` inline when CMake lives in a subdirectory. For configure/Make projects, configure with Clang and wrap Make with `bear --output "$BUILD_DIR/compile_commands.json"`. Build only the library target when optional tools are broken or irrelevant.

## 3. Keep the definition lean

Disable tests, benchmarks, examples, docs, bindings, and optional codecs unless they generate source required by the main library. Add a system package to `project-build.yml` only when the build genuinely needs it and no smaller project-local option exists.

## 4. Validate locally

Parse all YAML files and syntax-check each embedded build command:

```bash
for config in .github/projects/*.yaml; do
  ruby -ryaml -e 'data=YAML.safe_load_file(ARGV[0]); abort unless data["name"] && data["repoUrl"] && data["build"]' "$config"
  build=$(ruby -ryaml -e 'puts YAML.safe_load_file(ARGV[0]).fetch("build")' "$config")
  bash -n -c "$build"
done
```

Run actionlint and the repository checks from `AGENTS.md`. Confirm project discovery returns the expected count.

## 5. Validate on GitHub

Publish a focused draft PR. Inspect the named matrix job for the new project. Fix project-specific failures in its YAML instead of adding special cases to orchestration. Require all of these to pass:

- project build and SCIP generation
- project fragment upload
- final site assembly
- Cloudflare file-count and file-size checks
- Chrome source-render smoke test

After merging, let the main workflow produce `code-browser-state`. Confirm the production catalog contains the project and source text renders before declaring completion.

The planner skips any upstream commit already present in the production catalog. Use the manual `force_rebuild` workflow input only when converter or build-definition changes must regenerate an unchanged upstream commit.
