# cargo-mujina 🦡

A Cargo subcommand to temporarily patch workspace dependencies for local plugin development.

[![Crates.io](https://img.shields.io/crates/v/mujina.svg)](https://crates.io/crates/mujina)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

`cargo-mujina` is designed specifically to streamline **local plugin development** across Rust workspaces. It safely and temporarily patches your `Cargo.toml` to point to local plugin paths or Git branches, executes your specified `cargo` command, and keeps your original workspace configuration safe.

## Naming

“Mujina” is a Japanese yokai often confused with raccoon dogs or badgers, believed to be a creature capable of shapeshifting. Additionally, the proverb “Onaji ana no mujina (same hole, same mujina)” means that even if appearances or origins differ, the essence remains the same.

This tool works in a similar way: it merely temporarily alters the “appearance” of references such as `path` or `git`, without changing the essential behavior of the crate itself (how it is treated as a dependency). The name is derived from this concept.

## Why?

When building plugin architectures in Rust, developing and testing plugins locally alongside a host application requires tedious and error-prone `Cargo.toml` tweaks (such as adding `path = "..."`, editing `[workspace.dependencies]`, or setting up `[patch]` sections). 

`cargo-mujina` automates this entire local plugin development workflow. It dynamically patches dependencies, auto-discovers plugin crates, and patches transitive Git dependencies back to your local host crate—all without polluting your git working tree with temporary paths.

## Intended Use

The primary purpose is an inventory-based plugin system. (Dynamically importing crates that match [package.metadata.plugin].prefix from a workspace in a different repository into [workspace.dependencies].)

However, since the mechanism itself is a general-purpose process that “temporarily swaps the dependency resolution source between two workspaces that have workspace member crates with the same name,” it can also be used in other scenarios such as:

* A development loop where you build and test downstream projects while making local modifications to upstream libraries (temporarily swap using `-W ‘{ path = “../core” }’`; since it’s rebuilt from Cargo.toml.org each time, it’s easy to revert to the original state with `cargo mujina restore`)
* Bisecting a regression caused by a specific commit in a dependency crate (repeat `cargo mujina test` while changing `-W ‘{ git = “...”, tag = “...” }’`)
* Temporary bulk replacement with a patched fork for vulnerability fixes, etc.
* Local pseudo-merge in cases where repositories are split for ownership reasons but development is unified (the plugin system is a type of this)

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
1. **Workspace Validation**: Ensures both your current directory and the target `-W` directory are Cargo workspaces.
1. **Dependency Replacement**: Finds matching crates between the workspaces and replaces their entries in `[workspace.dependencies]` with the specified local path or git source.
1. **Git Patch Injection**: If the target workspace depends on any of your current workspace's crates via Git, it automatically appends a `[patch."<git-url>"]` section pointing back to your local crates.
1. **Command Execution**: Runs the specified cargo command (e.g., `cargo build`) with any trailing arguments you provided.

### Plugin Auto-Discovery

If you are developing a plugin system, you can configure `cargo-mujina` to automatically add new plugins from the target workspace even if they aren't already listed in your dependencies. 

Add the following to your root `Cargo.toml`:

```toml
[package.metadata.plugin]
prefix = "my-plugin-"
```

When you run `cargo mujina`, any crate in the target workspace whose name starts with `my-plugin-` will be:
1. Added to your `[workspace.dependencies]`.
1. Appended to your root-level `[dependencies]` as `{ workspace = true }`.

## Restoring `Cargo.toml`

`cargo-mujina` intentionally leaves the modified `Cargo.toml` in place so your language server (like `rust-analyzer`) can utilize the patched local dependencies.

When you are done with local testing, restore your original configuration:

```sh
cargo mujina restore
```

## For Plugin Developers: Precautions When Using `inventory`

Even if you add a crate to register a plugin using `inventory::submit!`, it won’t work on its own. If that crate isn’t `use`d anywhere, the linker will determine that it is “unused” and remove the entire crate—including its static initializers—which may prevent the registration with `inventory` from being executed at all.

Therefore, the following two steps are required:

1. Insert `use <plugin_crate> as _;` into `build.rs`. `cargo_metadata` reads the [dependencies] of the root crate, generates a `use` statement for each crate name that matches the prefix (converting hyphens to underscores), and writes them to `$OUT_DIR/generated_plugins.rs`.
1. Include that file using `include!` in `main.rs`.

```rusr
include!(concat!(env!("OUT_DIR"), "/generated_plugins.rs"));
```

You only need to insert one line, but if you forget to do so, the plugin will not work for the reasons mentioned above.

For specific implementation examples, please refer to sample/build.rs and sample.main.rs.

> **Note:** While the prefix check in `build.rs` includes a fallback (“{package name}-plugin-”) for cases where `[package.metadata.plugin].prefix` is not set, the automatic addition of new crates by `cargo-mujina` itself (`apply_patches`) only works when the prefix is explicitly set.

## License

Licensed under the Apache License, Version 2.0.
