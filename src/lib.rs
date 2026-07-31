use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Result of parsing an inline table passed via `-W` / `--with`.
/// Requirement: since the style is "paste the value given via the argument into Cargo.toml
/// without modification," for a git spec we keep the raw table (`raw_table`) as-is and paste it
/// later without reconstructing it.
/// The path spec is the sole exception: only when given as an absolute path is it converted to a
/// path relative to `current_dir` (see [`normalize_with_spec`] for details).
#[derive(Debug, Clone)]
pub enum WithSpec {
    Path {
        /// The path string as specified by the user.
        /// Right after parsing, this is the input as-is (unmodified), but inside `apply_patches`
        /// it is converted to a path relative to `current_dir` only if it was given as an
        /// absolute path (see [`normalize_with_spec`]).
        raw_path: String,
    },
    Git {
        /// The table exactly as specified by the user (unmodified; also keeps any keys other
        /// than git / branch / tag)
        raw_table: toml::Table,
        url: String,
        branch: Option<String>,
        tag: Option<String>,
    },
}

pub fn parse_with_option(input: &str) -> Result<WithSpec> {
    let trimmed = input.trim();
    let inline_str = if trimmed.starts_with('{') && trimmed.ends_with('}') {
        trimmed.to_string()
    } else {
        format!("{{{}}}", trimmed)
    };

    let dummy_toml = format!("spec = {}", inline_str);
    let parsed: toml::Table = toml::from_str(&dummy_toml)
        .with_context(|| format!("Failed to parse the TOML value of '-W / --with': '{}'", input))?;

    let spec_table = parsed
        .get("spec")
        .and_then(|v| v.as_table())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Invalid inline table format: '{}'", input))?;

    let path = spec_table.get("path").and_then(|v| v.as_str()).map(String::from);
    let git = spec_table.get("git").and_then(|v| v.as_str()).map(String::from);

    match (path, git) {
        (Some(raw_path), None) => Ok(WithSpec::Path { raw_path }),
        (None, Some(url)) => {
            let branch = spec_table.get("branch").and_then(|v| v.as_str()).map(String::from);
            let tag = spec_table.get("tag").and_then(|v| v.as_str()).map(String::from);
            Ok(WithSpec::Git {
                raw_table: spec_table,
                url,
                branch,
                tag,
            })
        }
        (Some(_), Some(_)) => {
            bail!("Cannot specify both 'path' and 'git' in a '-W / --with' spec: '{}'", input)
        }
        (None, None) => {
            bail!("A '-W / --with' spec must include either 'path' or 'git': '{}'", input)
        }
    }
}

pub fn toml_value_to_edit_value(val: &toml::Value) -> Result<toml_edit::Value> {
    match val {
        toml::Value::String(s) => Ok(toml_edit::Value::from(s.as_str())),
        toml::Value::Integer(i) => Ok(toml_edit::Value::from(*i)),
        toml::Value::Float(f) => Ok(toml_edit::Value::from(*f)),
        toml::Value::Boolean(b) => Ok(toml_edit::Value::from(*b)),
        _ => bail!("Unsupported TOML value type: {:?}", val),
    }
}

pub fn to_inline_table(table: &toml::Table) -> Result<toml_edit::InlineTable> {
    let mut inline = toml_edit::InlineTable::new();
    for (k, v) in table {
        let edit_val = toml_value_to_edit_value(v)?;
        inline.insert(k, edit_val);
    }
    Ok(inline)
}

pub fn ensure_backup(current_dir: &Path) -> Result<()> {
    let cargo_toml = current_dir.join("Cargo.toml");
    let cargo_toml_org = current_dir.join("Cargo.toml.org");

    if !cargo_toml.exists() && !cargo_toml_org.exists() {
        bail!(
            "Neither Cargo.toml nor Cargo.toml.org exists in the current directory: {}",
            current_dir.display()
        );
    }

    if !cargo_toml_org.exists() {
        std::fs::copy(&cargo_toml, &cargo_toml_org).with_context(|| {
            format!(
                "Failed to create a backup from {} to {}",
                cargo_toml.display(),
                cargo_toml_org.display()
            )
        })?;
    }

    Ok(())
}

pub fn get_package_name_in_dir(target_dir: &Path) -> Result<String> {
    let cargo_toml_path = target_dir.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        bail!("Cargo.toml does not exist at the specified location: {}", cargo_toml_path.display());
    }

    let content = std::fs::read_to_string(&cargo_toml_path)?;
    let doc: toml::Table = toml::from_str(&content)
        .with_context(|| format!("Failed to parse Cargo.toml: {}", cargo_toml_path.display()))?;

    let pkg = doc.get("package").and_then(|p| p.as_table()).ok_or_else(|| {
        anyhow::anyhow!(
            "Cargo.toml at the specified location ({}) does not have a [package] section",
            target_dir.display()
        )
    })?;

    let name = pkg
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml's [package] does not specify a 'name'"))?;

    Ok(name.to_string())
}

/// Information for a single workspace member crate.
/// `rel_path` is the path relative to the workspace root (the pattern written in `members`,
/// expanded as-is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMember {
    pub name: String,
    pub rel_path: String,
}

/// Validates that the Cargo.toml at `manifest_path` is workspace-configured (has a `[workspace]`),
/// and returns its list of member crates (name + path relative to the root).
/// Errors if `[workspace]` is not present (this also covers the validation for requirement 1 /
/// requirement 3).
pub fn parse_workspace_members(manifest_path: &Path, root_dir: &Path) -> Result<Vec<WorkspaceMember>> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let doc: toml::Table = toml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", manifest_path.display()))?;

    let workspace = doc.get("workspace").and_then(|w| w.as_table()).ok_or_else(|| {
        anyhow::anyhow!(
            "{} is not a workspace-configured Cargo.toml (no [workspace] section found at the top level)",
            root_dir.display()
        )
    })?;

    let member_patterns: Vec<String> = workspace
        .get("members")
        .and_then(|m| m.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let exclude_patterns: Vec<String> = workspace
        .get("exclude")
        .and_then(|m| m.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let mut members = Vec::new();

    for pattern in &member_patterns {
        for rel_path in expand_member_pattern(root_dir, pattern)? {
            if exclude_patterns.contains(&rel_path) {
                continue;
            }

            let member_cargo_toml = root_dir.join(&rel_path).join("Cargo.toml");
            if !member_cargo_toml.exists() {
                continue;
            }

            if let Ok(name) = get_package_name_in_dir(&root_dir.join(&rel_path)) {
                members.push(WorkspaceMember {
                    name,
                    rel_path: rel_path.replace('\\', "/"),
                });
            }
        }
    }

    Ok(members)
}

/// Expands a pattern written in `members`. If it ends with `/*`, expands to the immediate child
/// directories under that directory; otherwise treats the pattern itself as a single member
/// (more complex globs such as `**` are not supported).
fn expand_member_pattern(root_dir: &Path, pattern: &str) -> Result<Vec<String>> {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        let base_dir = root_dir.join(prefix);
        let mut result = Vec::new();

        if base_dir.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&base_dir)
                .with_context(|| format!("Failed to read {}", base_dir.display()))?
                .filter_map(|e| e.ok())
                .collect();
            entries.sort_by_key(|e| e.file_name());

            for entry in entries {
                if entry.path().is_dir() {
                    result.push(format!("{}/{}", prefix, entry.file_name().to_string_lossy()));
                }
            }
        }

        Ok(result)
    } else {
        Ok(vec![pattern.to_string()])
    }
}

/// Table paths (dot-separated) that may be searched for dependencies.
const DEP_TABLE_PATHS: [&[&str]; 4] = [
    &["dependencies"],
    &["dev-dependencies"],
    &["build-dependencies"],
    &["workspace", "dependencies"],
];

#[derive(Debug, Clone)]
pub struct GitDependencyRef {
    pub crate_name: String,
    pub git_url: String,
}

/// Scans the Cargo.toml of `root_dir` (the argument-side project's root) and all of its members,
/// looking for places where a dependency whose name is in `candidate_names` is specified via
/// `git` (requirement 5).
pub fn find_git_dependency_refs(
    root_dir: &Path,
    members: &[WorkspaceMember],
    candidate_names: &HashSet<String>,
) -> Result<Vec<GitDependencyRef>> {
    let mut manifests = vec![root_dir.join("Cargo.toml")];
    for member in members {
        manifests.push(root_dir.join(&member.rel_path).join("Cargo.toml"));
    }

    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for manifest_path in manifests {
        if !manifest_path.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
        let doc: toml::Table = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", manifest_path.display()))?;

        for table_path in DEP_TABLE_PATHS {
            let mut current: Option<&toml::Table> = Some(&doc);
            for key in table_path {
                current = current.and_then(|t| t.get(*key)).and_then(|v| v.as_table());
            }

            if let Some(dep_table) = current {
                for (dep_name, dep_value) in dep_table {
                    if !candidate_names.contains(dep_name) {
                        continue;
                    }

                    if let Some(git_url) = dep_value.as_table().and_then(|t| t.get("git")).and_then(|g| g.as_str()) {
                        if seen.insert((dep_name.clone(), git_url.to_string())) {
                            result.push(GitDependencyRef {
                                crate_name: dep_name.clone(),
                                git_url: git_url.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(result)
}

/// A guard that keeps a git-clone temporary directory alive until processing finishes.
#[allow(dead_code)]
enum RootGuard {
    None,
    Temp(tempfile::TempDir),
}

/// Computes, relative to `base_dir`, the relative path string to the absolute path `abs_path`.
/// This is a purely lexical computation (it only cancels `..` out against the preceding normal
/// component) and never touches the filesystem, e.g. for symlink resolution.
fn to_relative_path(abs_path: &Path, base_dir: &Path) -> String {
    use std::path::Component;

    // Lexically fold `.` / `..`, cancelling `..` out against the preceding normal component.
    fn clean_components(path: &Path) -> Vec<Component<'_>> {
        let mut out: Vec<Component> = Vec::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir if matches!(out.last(), Some(Component::Normal(_))) => {
                    out.pop();
                }
                other => out.push(other),
            }
        }
        out
    }

    let target_components = clean_components(abs_path);
    let base_components = clean_components(base_dir);

    let common_len = target_components
        .iter()
        .zip(base_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = PathBuf::new();
    for _ in &base_components[common_len..] {
        result.push("..");
    }
    for component in &target_components[common_len..] {
        result.push(component.as_os_str());
    }

    if result.as_os_str().is_empty() {
        result.push(".");
    }

    result.to_string_lossy().replace('\\', "/")
}

/// Returns a new `WithSpec` with the path converted to one relative to `current_dir`, but only
/// when `spec` is a `WithSpec::Path` pointing at an absolute path. For a relative-path spec or a
/// `WithSpec::Git` spec, this returns an unmodified clone.
///
/// This way, regardless of whether `-W path=...` is given as an absolute or a relative path on
/// the CLI side, the generated Cargo.toml always ends up with a path relative to current_dir.
fn normalize_with_spec(spec: &WithSpec, current_dir: &Path) -> Result<WithSpec> {
    match spec {
        WithSpec::Path { raw_path } => {
            let path = Path::new(raw_path);
            if !path.is_absolute() {
                return Ok(spec.clone());
            }

            Ok(WithSpec::Path {
                raw_path: to_relative_path(path, current_dir),
            })
        }
        WithSpec::Git { .. } => Ok(spec.clone()),
    }
}

/// Resolves the root directory of the argument-side project from a `-W` spec.
/// For a path spec this is a simple join with the current directory; for a git spec this is
/// wherever it was shallow-cloned to.
fn resolve_with_root(spec: &WithSpec, current_dir: &Path) -> Result<(PathBuf, RootGuard)> {
    match spec {
        WithSpec::Path { raw_path } => {
            let dir = current_dir.join(raw_path);
            Ok((dir, RootGuard::None))
        }
        WithSpec::Git { url, branch, tag, .. } => {
            let temp_dir = tempfile::Builder::new()
                .prefix("mujina-git-")
                .tempdir()
                .context("Failed to create a temporary directory")?;

            let mut cmd = std::process::Command::new("git");
            cmd.arg("clone").arg("--depth").arg("1");

            if let Some(branch) = branch {
                cmd.arg("--branch").arg(branch);
            } else if let Some(tag) = tag {
                cmd.arg("--branch").arg(tag);
            }

            cmd.arg(url).arg(temp_dir.path());

            let output = cmd
                .output()
                .with_context(|| format!("Failed to run git clone: {}", url))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("git clone failed ({}) :\n{}", url, stderr);
            }

            let path = temp_dir.path().to_path_buf();
            Ok((path, RootGuard::Temp(temp_dir)))
        }
    }
}

/// Requirement 2 & requirement 4: builds the value to insert into `[workspace.dependencies]`
/// without any modification (using generation rules only).
/// - For a path spec: raw_path (already converted to a path relative to current_dir by
///   `normalize_with_spec` beforehand, if it was given as an absolute path) is simply
///   string-concatenated with the member's relative path within the argument-side workspace;
///   no filesystem-based recomputation such as canonicalize happens here.
/// - For a git spec: the table specified by the user is pasted as-is.
fn build_replacement_value(spec: &WithSpec, target_member: &WorkspaceMember) -> Result<toml_edit::Value> {
    match spec {
        WithSpec::Path { raw_path } => {
            let joined = if target_member.rel_path.is_empty() || target_member.rel_path == "." {
                raw_path.clone()
            } else {
                format!("{}/{}", raw_path.trim_end_matches('/'), target_member.rel_path)
            };

            let mut table = toml_edit::InlineTable::new();
            table.insert("path", toml_edit::Value::from(joined.as_str()));
            Ok(toml_edit::Value::InlineTable(table))
        }
        WithSpec::Git { raw_table, .. } => {
            let inline = to_inline_table(raw_table)?;
            Ok(toml_edit::Value::InlineTable(inline))
        }
    }
}

/// Replaces the corresponding crate's entry in `[workspace.dependencies]` with a new value
/// (adds it if it doesn't exist).
fn set_workspace_dependency(doc: &mut toml_edit::DocumentMut, crate_name: &str, value: toml_edit::Value) -> Result<()> {
    let workspace_item = doc
        .get_mut("workspace")
        .ok_or_else(|| anyhow::anyhow!("[workspace] section not found"))?;
    let workspace_table = workspace_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[workspace] is not a table"))?;

    let deps_item = workspace_table
        .entry("dependencies")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let deps_table = deps_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[workspace.dependencies] is not a table"))?;

    deps_table.insert(crate_name, toml_edit::value(value));
    Ok(())
}

/// Requirement 5: appends the upstream (current-directory-side) path to the
/// `[patch."<git url>"]` section.
fn apply_git_patches(
    doc: &mut toml_edit::DocumentMut,
    git_patches: BTreeMap<String, Vec<(String, String)>>,
) -> Result<()> {
    if git_patches.is_empty() {
        return Ok(());
    }

    let patch_item = doc.entry("patch").or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let patch_table = patch_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[patch] in Cargo.toml.org is not a table"))?;
    patch_table.set_implicit(true);

    for (url, entries) in git_patches {
        let url_item = patch_table
            .entry(&url)
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        let url_table = url_item
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("[patch.\"{}\"] is not a table", url))?;

        for (crate_name, rel_path) in entries {
            let mut inline = toml_edit::InlineTable::new();
            inline.insert("path", toml_edit::Value::from(rel_path));
            url_table.insert(&crate_name, toml_edit::value(toml_edit::Value::InlineTable(inline)));
        }
    }

    Ok(())
}

/// Requirement 6: reads `[package.metadata.plugin].prefix` from the current directory's
/// Cargo.toml (org). Returns `None` (the plugin auto-add feature is disabled) if this
/// section/key is not present.
fn read_plugin_prefix(org_content: &str) -> Result<Option<String>> {
    let doc: toml::Table = toml::from_str(org_content)
        .context("Failed to parse Cargo.toml.org (while reading plugin configuration)")?;

    let prefix = doc
        .get("package")
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.as_table())
        .and_then(|m| m.get("plugin"))
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("prefix"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(prefix)
}

/// Requirement 6: appends `<crate_name> = { workspace = true }` to the root-level
/// `[dependencies]` (for a crate that is newly added because its name matches
/// `[package.metadata.plugin].prefix`, rather than being an existing member).
fn set_root_dependency_as_workspace(doc: &mut toml_edit::DocumentMut, crate_name: &str) -> Result<()> {
    let deps_item = doc.entry("dependencies").or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let deps_table = deps_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[dependencies] is not a table"))?;

    let mut inline = toml_edit::InlineTable::new();
    inline.insert("workspace", toml_edit::Value::from(true));
    deps_table.insert(crate_name, toml_edit::value(toml_edit::Value::InlineTable(inline)));

    Ok(())
}

/// The main entry point. `current_dir` is the directory the tool was invoked from, and it must
/// have a workspace-configured Cargo.toml (requirement 1). `with_specs` is the list of specs
/// passed via `-W`.
pub fn apply_patches(current_dir: &Path, with_specs: &[WithSpec]) -> Result<()> {
    ensure_backup(current_dir)?;

    let cargo_toml_org = current_dir.join("Cargo.toml.org");
    let cargo_toml = current_dir.join("Cargo.toml");

    let org_content = std::fs::read_to_string(&cargo_toml_org)
        .with_context(|| format!("Failed to read {}", cargo_toml_org.display()))?;

    // Requirement 1: the current directory must be workspace-configured
    let current_members = parse_workspace_members(&cargo_toml_org, current_dir)
        .context("Failed to validate the current directory's Cargo.toml")?;
    let current_member_names: HashSet<String> = current_members.iter().map(|m| m.name.clone()).collect();

    // Requirement 6: auto-add new crates matching [package.metadata.plugin].prefix (optional configuration)
    let plugin_prefix = read_plugin_prefix(&org_content)?;

    let mut doc: toml_edit::DocumentMut = org_content
        .parse()
        .with_context(|| format!("Failed to parse {}", cargo_toml_org.display()))?;

    // url -> [(crate_name, path relative to upstream)]
    let mut git_patches: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

    for spec in with_specs {
        // Only when the path spec is an absolute path do we convert it here to a path relative
        // to current_dir. From here on, this normalized spec is used both to resolve root_dir
        // and to build the replacement value.
        let spec = normalize_with_spec(spec, current_dir)?;

        let (root_dir, _root_guard) = resolve_with_root(&spec, current_dir)?;

        // Requirement 3: the argument side must also have a workspace-configured Cargo.toml
        // directly under its root
        let target_manifest = root_dir.join("Cargo.toml");
        let target_members = parse_workspace_members(&target_manifest, &root_dir)
            .with_context(|| format!("Failed to validate {}", root_dir.display()))?;

        // Requirement 4 & requirement 6: choose what to replace/add in [workspace.dependencies]
        // - Requirement 4: replace an existing member whose name matches, as before
        // - Requirement 6: even if it's not an existing member, newly add an argument-side
        //   member whose name matches [package.metadata.plugin].prefix (and also append it to
        //   the root-level [dependencies] as `{ workspace = true }`)
        for target_member in &target_members {
            let is_existing_member = current_member_names.contains(&target_member.name);
            let matches_plugin_prefix = plugin_prefix
                .as_deref()
                .map(|prefix| target_member.name.starts_with(prefix))
                .unwrap_or(false);

            if !is_existing_member && !matches_plugin_prefix {
                continue;
            }

            let new_value = build_replacement_value(&spec, target_member)?;
            set_workspace_dependency(&mut doc, &target_member.name, new_value)?;

            if !is_existing_member && matches_plugin_prefix {
                set_root_dependency_as_workspace(&mut doc, &target_member.name)?;
            }
        }

        // Requirement 5: if the argument side (root or a member) references, via a git spec, a
        // crate with the same name as a current-directory member, add a patch pointing at the
        // upstream (the real thing, on the current-directory side)
        let git_refs = find_git_dependency_refs(&root_dir, &target_members, &current_member_names)?;
        for git_ref in git_refs {
            if let Some(own_member) = current_members.iter().find(|m| m.name == git_ref.crate_name) {
                git_patches
                    .entry(git_ref.git_url.clone())
                    .or_default()
                    .push((git_ref.crate_name.clone(), own_member.rel_path.clone()));
            }
        }
    }

    apply_git_patches(&mut doc, git_patches)?;

    std::fs::write(&cargo_toml, doc.to_string())
        .with_context(|| format!("Failed to write {}", cargo_toml.display()))?;

    Ok(())
}

/// Requirement: Restores Cargo.toml from Cargo.toml.org and removes the backup file.
/// Returns `Ok(true)` if restored, or `Ok(false)` if no backup existed.
pub fn restore_backup(current_dir: &Path) -> Result<bool> {
    let cargo_toml = current_dir.join("Cargo.toml");
    let cargo_toml_org = current_dir.join("Cargo.toml.org");

    // Return an error if neither exists
    if !cargo_toml.exists() && !cargo_toml_org.exists() {
        bail!(
            "Neither Cargo.toml nor Cargo.toml.org exists in the current directory: {}",
            current_dir.display()
        );
    }

    // If Cargo.toml exists but there is no backup (.org), no restoration is needed (already original).
    if !cargo_toml_org.exists() {
        return Ok(false);
    }

    // Include fallback handling in case rename fails due to environment issues (OS, permissions, etc.)
    if std::fs::rename(&cargo_toml_org, &cargo_toml).is_err() {
        std::fs::copy(&cargo_toml_org, &cargo_toml).with_context(|| {
            format!(
                "Failed to copy {} to {}",
                cargo_toml_org.display(),
                cargo_toml.display()
            )
        })?;
        std::fs::remove_file(&cargo_toml_org).with_context(|| {
            format!("Failed to remove backup file {}", cargo_toml_org.display())
        })?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn test_ensure_backup() {
        let dir = tempdir().unwrap();
        let current = dir.path();
        let cargo_toml = current.join("Cargo.toml");
        let cargo_toml_org = current.join("Cargo.toml.org");

        std::fs::write(&cargo_toml, "original content").unwrap();

        ensure_backup(current).unwrap();
        assert!(cargo_toml_org.exists());
        assert_eq!(std::fs::read_to_string(&cargo_toml_org).unwrap(), "original content");

        std::fs::write(&cargo_toml, "modified content").unwrap();

        ensure_backup(current).unwrap();
        assert_eq!(std::fs::read_to_string(&cargo_toml_org).unwrap(), "original content");
    }

    #[test]
    fn test_get_package_name_no_package_error() {
        let dir = tempdir().unwrap();
        let ws_dir = dir.path();

        let ws_cargo = ws_dir.join("Cargo.toml");
        std::fs::write(
            &ws_cargo,
            r#"[workspace]
members = ["crates/*"]
"#,
        )
        .unwrap();

        let res = get_package_name_in_dir(ws_dir);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("[package] section"));
    }

    #[test]
    fn test_get_package_name_success() {
        let dir = tempdir().unwrap();
        let crate_dir = dir.path();

        let cargo_toml = crate_dir.join("Cargo.toml");
        std::fs::write(
            &cargo_toml,
            r#"[package]
name = "my-lib"
version = "0.1.0"
"#,
        )
        .unwrap();

        let name = get_package_name_in_dir(crate_dir).unwrap();
        assert_eq!(name, "my-lib");
    }

    #[test]
    fn test_parse_with_option_path() {
        let spec = parse_with_option(r#"{ path = "../foo" }"#).unwrap();
        match spec {
            WithSpec::Path { raw_path } => assert_eq!(raw_path, "../foo"),
            _ => panic!("expected Path variant"),
        }
    }

    #[test]
    fn test_parse_with_option_git() {
        let spec = parse_with_option(r#"{ git = "https://github.com/x/y.git", branch = "main" }"#).unwrap();
        match spec {
            WithSpec::Git { url, branch, .. } => {
                assert_eq!(url, "https://github.com/x/y.git");
                assert_eq!(branch.as_deref(), Some("main"));
            }
            _ => panic!("expected Git variant"),
        }
    }

    #[test]
    fn test_parse_with_option_requires_path_or_git() {
        let res = parse_with_option(r#"{ branch = "main" }"#);
        assert!(res.is_err());
    }

    #[test]
    fn test_parse_workspace_members_requires_workspace_section() {
        let dir = tempdir().unwrap();
        let cargo_toml = dir.path().join("Cargo.toml");
        write_file(
            &cargo_toml,
            r#"[package]
name = "not-a-workspace"
version = "0.1.0"
"#,
        );

        let res = parse_workspace_members(&cargo_toml, dir.path());
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("[workspace]"));
    }

    #[test]
    fn test_parse_workspace_members_with_glob() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        write_file(
            &root.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/*"]
"#,
        );
        write_file(
            &root.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.1.0"
"#,
        );
        write_file(
            &root.join("crates/b_crate/Cargo.toml"),
            r#"[package]
name = "b_crate"
version = "0.1.0"
"#,
        );

        let members = parse_workspace_members(&root.join("Cargo.toml"), root).unwrap();
        let names: Vec<_> = members.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"a_crate"));
        assert!(names.contains(&"b_crate"));

        let a = members.iter().find(|m| m.name == "a_crate").unwrap();
        assert_eq!(a.rel_path, "crates/a_crate");
    }

    #[test]
    fn test_find_git_dependency_refs() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        write_file(
            &root.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/consumer"]

[workspace.dependencies]
a_crate = { git = "https://github.com/xxx/yyy.git" }
"#,
        );
        write_file(
            &root.join("crates/consumer/Cargo.toml"),
            r#"[package]
name = "consumer"
version = "0.1.0"

[dependencies]
a_crate = { workspace = true }
"#,
        );

        let members = parse_workspace_members(&root.join("Cargo.toml"), root).unwrap();
        let mut candidates = HashSet::new();
        candidates.insert("a_crate".to_string());

        let refs = find_git_dependency_refs(root, &members, &candidates).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].crate_name, "a_crate");
        assert_eq!(refs[0].git_url, "https://github.com/xxx/yyy.git");
    }

    /// Requirement 1: it's an error if the current directory is not workspace-configured
    #[test]
    fn test_apply_patches_requires_workspace_in_current_dir() {
        let dir = tempdir().unwrap();
        let app_dir = dir.path();

        write_file(
            &app_dir.join("Cargo.toml"),
            r#"[package]
name = "app"
version = "0.1.0"
"#,
        );

        let res = apply_patches(app_dir, &[]);
        assert!(res.is_err());
        assert!(format!("{:?}", res.unwrap_err()).contains("[workspace]"));
    }

    /// Requirement 3: it's an error if the argument side is not workspace-configured
    #[test]
    fn test_apply_patches_requires_workspace_in_target() {
        let dir = tempdir().unwrap();
        let app_dir = dir.path().join("app");
        let other_dir = dir.path().join("other");

        write_file(
            &app_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate"]
"#,
        );
        write_file(
            &app_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.1.0"
"#,
        );

        // The argument side is not a workspace, just a plain package
        write_file(
            &other_dir.join("Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.2.0"
"#,
        );

        let spec = WithSpec::Path {
            raw_path: "../other".to_string(),
        };

        let res = apply_patches(&app_dir, &[spec]);
        assert!(res.is_err());
        assert!(format!("{:?}", res.unwrap_err()).contains("[workspace]"));
    }

    #[test]
    fn test_to_relative_path_basic() {
        let base = Path::new("/home/user/app");
        let target = Path::new("/home/user/other");
        assert_eq!(to_relative_path(target, base), "../other");
    }

    #[test]
    fn test_to_relative_path_nested() {
        let base = Path::new("/home/user/app/crates/a");
        let target = Path::new("/home/user/other");
        assert_eq!(to_relative_path(target, base), "../../../other");
    }

    #[test]
    fn test_to_relative_path_descendant() {
        let base = Path::new("/home/user/app");
        let target = Path::new("/home/user/app/crates/a");
        assert_eq!(to_relative_path(target, base), "crates/a");
    }

    #[test]
    fn test_to_relative_path_same_dir() {
        let base = Path::new("/home/user/app");
        let target = Path::new("/home/user/app");
        assert_eq!(to_relative_path(target, base), ".");
    }

    #[test]
    fn test_to_relative_path_collapses_redundant_segments() {
        // Even if the absolute-path side contains redundant `..` / `.`, they get lexically
        // folded away
        let base = Path::new("/home/user/app");
        let target = Path::new("/home/user/app/../other/./sub");
        assert_eq!(to_relative_path(target, base), "../other/sub");
    }

    #[test]
    fn test_normalize_with_spec_leaves_relative_path_untouched() {
        let spec = WithSpec::Path {
            raw_path: "../other".to_string(),
        };
        let normalized = normalize_with_spec(&spec, Path::new("/home/user/app")).unwrap();
        match normalized {
            WithSpec::Path { raw_path } => assert_eq!(raw_path, "../other"),
            _ => panic!("expected Path variant"),
        }
    }

    #[test]
    fn test_normalize_with_spec_converts_absolute_path() {
        let spec = WithSpec::Path {
            raw_path: "/home/user/other".to_string(),
        };
        let normalized = normalize_with_spec(&spec, Path::new("/home/user/app")).unwrap();
        match normalized {
            WithSpec::Path { raw_path } => assert_eq!(raw_path, "../other"),
            _ => panic!("expected Path variant"),
        }
    }

    /// Even when specifying -W path=... as an absolute path, it should be written into
    /// Cargo.toml as a path relative to current_dir
    #[test]
    fn test_apply_patches_converts_absolute_path_to_relative() {
        let dir = tempdir().unwrap();
        let app_dir = dir.path().join("app");
        let other_dir = dir.path().join("other");

        write_file(
            &app_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate"]

[workspace.dependencies]
a_crate = { path = "crates/a_crate" }
"#,
        );
        write_file(
            &app_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.1.0"
"#,
        );

        write_file(
            &other_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate"]
"#,
        );
        write_file(
            &other_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.2.0"
"#,
        );

        // other_dir is already an absolute path (from tempdir), so use it as-is as an
        // absolute-path spec
        let spec = WithSpec::Path {
            raw_path: other_dir.to_string_lossy().to_string(),
        };

        apply_patches(&app_dir, &[spec]).unwrap();

        let updated = std::fs::read_to_string(app_dir.join("Cargo.toml")).unwrap();
        assert!(updated.contains(r#"a_crate = { path = "../other/crates/a_crate" }"#));
    }

    #[test]
    fn test_read_plugin_prefix_present() {
        let content = r#"[package]
name = "app"
version = "0.1.0"

[package.metadata.plugin]
prefix = "aralez-plugin-"

[workspace]
members = ["crates/*"]
"#;
        assert_eq!(read_plugin_prefix(content).unwrap(), Some("aralez-plugin-".to_string()));
    }

    #[test]
    fn test_read_plugin_prefix_absent() {
        let content = r#"[workspace]
members = ["crates/*"]
"#;
        assert_eq!(read_plugin_prefix(content).unwrap(), None);
    }

    /// Requirement 6: a crate on the argument side that matches
    /// [package.metadata.plugin].prefix and is not an existing member gets newly added to
    /// [workspace.dependencies], and is also appended to the root-level [dependencies] as
    /// `{ workspace = true }`
    #[test]
    fn test_apply_patches_adds_new_plugin_crate_matching_prefix() {
        let dir = tempdir().unwrap();
        let app_dir = dir.path().join("app");
        let other_dir = dir.path().join("other");

        write_file(
            &app_dir.join("Cargo.toml"),
            r#"[package]
name = "app"
version = "0.1.0"

[package.metadata.plugin]
prefix = "aralez-plugin-"

[workspace]
members = ["crates/a_crate"]
"#,
        );
        write_file(
            &app_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.1.0"
"#,
        );

        write_file(
            &other_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate", "crates/aralez-plugin-foo", "crates/unrelated"]
"#,
        );
        write_file(
            &other_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.2.0"
"#,
        );
        write_file(
            &other_dir.join("crates/aralez-plugin-foo/Cargo.toml"),
            r#"[package]
name = "aralez-plugin-foo"
version = "0.1.0"
"#,
        );
        write_file(
            &other_dir.join("crates/unrelated/Cargo.toml"),
            r#"[package]
name = "unrelated"
version = "0.1.0"
"#,
        );

        let spec = WithSpec::Path {
            raw_path: "../other".to_string(),
        };

        apply_patches(&app_dir, &[spec]).unwrap();

        let updated = std::fs::read_to_string(app_dir.join("Cargo.toml")).unwrap();

        // the existing member a_crate has its [workspace.dependencies] entry replaced, as before
        assert!(updated.contains(r#"a_crate = { path = "../other/crates/a_crate" }"#));

        // a new crate (matching the prefix) gets added to [workspace.dependencies]
        assert!(updated.contains(r#"aralez-plugin-foo = { path = "../other/crates/aralez-plugin-foo" }"#));

        // and it also gets appended to the root-level [dependencies] as { workspace = true }
        assert!(updated.contains(r#"aralez-plugin-foo = { workspace = true }"#));

        // a crate that neither matches the prefix nor is an existing member is not added
        assert!(!updated.contains("unrelated"));
    }

    /// When there is no prefix configuration ([package.metadata.plugin]), no new crate is
    /// auto-added (only existing members get replaced, as before)
    #[test]
    fn test_apply_patches_does_not_add_new_crate_without_plugin_config() {
        let dir = tempdir().unwrap();
        let app_dir = dir.path().join("app");
        let other_dir = dir.path().join("other");

        write_file(
            &app_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate"]
"#,
        );
        write_file(
            &app_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.1.0"
"#,
        );

        write_file(
            &other_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate", "crates/aralez-plugin-foo"]
"#,
        );
        write_file(
            &other_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.2.0"
"#,
        );
        write_file(
            &other_dir.join("crates/aralez-plugin-foo/Cargo.toml"),
            r#"[package]
name = "aralez-plugin-foo"
version = "0.1.0"
"#,
        );

        let spec = WithSpec::Path {
            raw_path: "../other".to_string(),
        };

        apply_patches(&app_dir, &[spec]).unwrap();

        let updated = std::fs::read_to_string(app_dir.join("Cargo.toml")).unwrap();
        assert!(updated.contains(r#"a_crate = { path = "../other/crates/a_crate" }"#));
        assert!(!updated.contains("aralez-plugin-foo"));
    }

    /// Requirement 2 & requirement 4: the argument-side path spec is reflected into
    /// [workspace.dependencies] unmodified
    #[test]
    fn test_apply_patches_replaces_workspace_dependency_with_path() {
        let dir = tempdir().unwrap();
        let app_dir = dir.path().join("app");
        let other_dir = dir.path().join("other");

        write_file(
            &app_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate"]

[workspace.dependencies]
a_crate = { path = "crates/a_crate" }
"#,
        );
        write_file(
            &app_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.1.0"
"#,
        );

        write_file(
            &other_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate"]
"#,
        );
        write_file(
            &other_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.2.0"
"#,
        );

        let spec = WithSpec::Path {
            raw_path: "../other".to_string(),
        };

        apply_patches(&app_dir, &[spec]).unwrap();

        let updated = std::fs::read_to_string(app_dir.join("Cargo.toml")).unwrap();
        assert!(updated.contains(r#"a_crate = { path = "../other/crates/a_crate" }"#));
    }

    /// Requirement 5: for a same-named crate that the argument-side project references via a
    /// git spec, a patch pointing at the upstream (current directory) side gets appended
    #[test]
    fn test_apply_patches_adds_git_patch_for_upstream_git_dependency() {
        let dir = tempdir().unwrap();
        let app_dir = dir.path().join("app");
        let other_dir = dir.path().join("other");

        write_file(
            &app_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/a_crate"]
"#,
        );
        write_file(
            &app_dir.join("crates/a_crate/Cargo.toml"),
            r#"[package]
name = "a_crate"
version = "0.1.0"
"#,
        );

        // The argument-side project itself doesn't have a_crate as a member, but internally
        // references a_crate via git
        write_file(
            &other_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/consumer"]

[workspace.dependencies]
a_crate = { git = "https://github.com/xxx/yyy.git" }
"#,
        );
        write_file(
            &other_dir.join("crates/consumer/Cargo.toml"),
            r#"[package]
name = "consumer"
version = "0.1.0"

[dependencies]
a_crate = { workspace = true }
"#,
        );

        let spec = WithSpec::Path {
            raw_path: "../other".to_string(),
        };

        apply_patches(&app_dir, &[spec]).unwrap();

        let updated = std::fs::read_to_string(app_dir.join("Cargo.toml")).unwrap();
        assert!(updated.contains(r#"[patch."https://github.com/xxx/yyy.git"]"#));
        assert!(updated.contains(r#"a_crate = { path = "crates/a_crate" }"#));
    }
    #[test]
    fn test_restore_backup() {
        let dir = tempdir().unwrap();
        let current = dir.path();
        let cargo_toml = current.join("Cargo.toml");
        let cargo_toml_org = current.join("Cargo.toml.org");

        std::fs::write(&cargo_toml_org, "original content").unwrap();
        std::fs::write(&cargo_toml, "modified content").unwrap();

        let restored = restore_backup(current).unwrap();
        assert!(restored);
        assert!(!cargo_toml_org.exists());
        assert_eq!(std::fs::read_to_string(&cargo_toml).unwrap(), "original content");

        // バックアップが無い状態でもう一度叩いた場合は false が返る
        let restored_again = restore_backup(current).unwrap();
        assert!(!restored_again);
    }
}
