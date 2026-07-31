# cargo-mujina

A Cargo subcommand to temporarily patch workspace dependencies for local plugin development.

[![Crates.io](https://img.shields.io/crates/v/mujina.svg)](https://crates.io/crates/mujina)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

`cargo-mujina` is designed specifically to streamline **local plugin development** across Rust workspaces. It safely and temporarily patches your `Cargo.toml` to point to local plugin paths or Git branches, executes your specified `cargo` command, and keeps your original workspace configuration safe.

## Why?

When building plugin architectures in Rust, developing and testing plugins locally alongside a host application requires tedious and error-prone `Cargo.toml` tweaks (such as adding `path = "..."`, editing `[workspace.dependencies]`, or setting up `[patch]` sections). 

`cargo-mujina` automates this entire local plugin development workflow. It dynamically patches dependencies, auto-discovers plugin crates, and patches transitive Git dependencies back to your local host crate—all without polluting your git working tree with temporary paths.

## Installation

```sh
cargo install --path .
```

## Usage

Use `cargo mujina` exactly like you would use standard Cargo commands (e.g., `build`, `test`, `check`, `run`), but pass the target repository you want to patch in via the `-W` or `--with` flag.

```sh
# Patch dependencies using a local workspace path and run `cargo build`
cargo mujina build -W '{ path = "../other-workspace" }'

# Pass additional arguments to cargo by placing them at the end
cargo mujina test -W '{ path = "../other-workspace" }' -- --nocapture

# Patch using a specific Git repository and branch
cargo mujina check -W '{ git = "https://github.com/example/repo.git", branch = "main" }'
```

### Supported Cargo Commands

`cargo-mujina` acts as a transparent wrapper for most standard `cargo` commands. The following commands are fully supported and will be forwarded to `cargo` after the temporary patches are applied:

- `bench`
- `build` (alias: `b`)
- `check` (alias: `c`)
- `clippy`
- `doc` (alias: `d`)
- `fetch`
- `metadata`
- `miri`
- `run` (alias: `r`)
- `rustc`
- `rustdoc`
- `test` (alias: `t`)
- `tree`
- `vendor`

### Mujina-Specific Commands

- `edit`: Applies the patches to `Cargo.toml` but exits without invoking any cargo command. Useful if you just want to update the `Cargo.toml` for your IDE.
- `restore`: Restores the original `Cargo.toml` from the `Cargo.toml.org` backup.

## How it works

When you run `cargo mujina <cmd> -W <spec>`, the following steps occur:

1. **Safety First (Backup)**: Checks for `Cargo.toml.org`. If it doesn't exist, it backs up your current `Cargo.toml` to `Cargo.toml.org`.
2. **Workspace Validation**: Ensures both your current directory and the target `-W` directory are Cargo workspaces.
3. **Dependency Replacement**: Finds matching crates between the workspaces and replaces their entries in `[workspace.dependencies]` with the specified local path or git source.
4. **Git Patch Injection**: If the target workspace depends on any of your current workspace's crates via Git, it automatically appends a `[patch."<git-url>"]` section pointing back to your local crates.
5. **Command Execution**: Runs the specified cargo command (e.g., `cargo build`) with any trailing arguments you provided.

### Plugin Auto-Discovery

If you are developing a plugin system, you can configure `cargo-mujina` to automatically add new plugins from the target workspace even if they aren't already listed in your dependencies. 

Add the following to your root `Cargo.toml`:

```toml
[package.metadata.plugin]
prefix = "my-plugin-"
```

When you run `cargo mujina`, any crate in the target workspace whose name starts with `my-plugin-` will be:
1. Added to your `[workspace.dependencies]`.
2. Appended to your root-level `[dependencies]` as `{ workspace = true }`.

## Restoring `Cargo.toml`

`cargo-mujina` intentionally leaves the modified `Cargo.toml` in place so your language server (like `rust-analyzer`) can utilize the patched local dependencies.

When you are done with local testing, restore your original configuration:

```sh
cargo mujina restore
```

## License

Licensed under the Apache License, Version 2.0.
