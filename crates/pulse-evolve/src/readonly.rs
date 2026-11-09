use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, FileType, Metadata};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TreeEntry {
    pub(crate) name: String,
    pub(crate) mode: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct TreeRoot {
    root: PathBuf,
}

impl TreeRoot {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            root: path.as_ref().to_path_buf(),
        })
    }

    fn join(&self, name: &str) -> PathBuf {
        if name == "." {
            self.root.clone()
        } else {
            self.root.join(name)
        }
    }
}

fn walk_names(root: &Path) -> Result<Vec<String>> {
    fn rec(root: &Path, rel: &str, out: &mut Vec<String>) -> Result<()> {
        out.push(if rel.is_empty() {
            ".".to_string()
        } else {
            rel.to_string()
        });
        let dir = if rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        let meta = fs::symlink_metadata(&dir)?;
        if !meta.is_dir() {
            return Ok(());
        }
        let mut entries = fs::read_dir(&dir)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().to_string()))
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort();
        for name in entries {
            let child = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            rec(root, &child, out)?;
        }
        Ok(())
    }
    let mut out = Vec::new();
    rec(root, "", &mut out)?;
    Ok(out)
}

fn escaping_symlink(_root: &TreeRoot, name: &str, target: &Path) -> bool {
    if target.is_absolute() || target.to_string_lossy().contains('\\') {
        return true;
    }
    let base = if name == "." {
        Path::new("")
    } else {
        Path::new(name).parent().unwrap_or(Path::new(""))
    };
    let joined = base.join(target);
    let mut depth = 0i64;
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            _ => return true,
        }
    }
    false
}

fn violation(reason: &str) -> anyhow::Error {
    anyhow!(crate::clones::ViolationError {
        reason: reason.to_string()
    })
}

#[cfg(unix)]
fn link_key(meta: &Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}
#[cfg(not(unix))]
fn link_key(_meta: &Metadata) -> (u64, u64) {
    (0, 0)
}

#[cfg(unix)]
fn link_count(meta: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.nlink()
}
#[cfg(not(unix))]
fn link_count(_meta: &Metadata) -> u64 {
    1
}

/// Every name under the root with its metadata, and how many names each
/// hardlinked file has inside the tree. A file whose every link is in the
/// tree cannot be reached from outside it - cargo uplifts build artifacts by
/// linking `target/debug/deps/libx-HASH.rmeta` to `target/debug/libx.rmeta` -
/// so only a file with a link the tree does not contain is an escape.
struct Walk {
    entries: Vec<(String, Metadata)>,
    inside: HashMap<(u64, u64), u64>,
}

impl Walk {
    fn of(root: &TreeRoot) -> Result<Self> {
        let mut entries = Vec::new();
        let mut inside: HashMap<(u64, u64), u64> = HashMap::new();
        for name in walk_names(&root.root)? {
            let info = fs::symlink_metadata(root.join(&name))?;
            if crate::process::multiple_links(&info) {
                *inside.entry(link_key(&info)).or_default() += 1;
            }
            entries.push((name, info));
        }
        Ok(Self { entries, inside })
    }

    /// A regular file with a hardlink that lives outside the tree.
    fn escaping_link(&self, info: &Metadata) -> bool {
        crate::process::multiple_links(info)
            && self.inside.get(&link_key(info)).copied().unwrap_or(0) < link_count(info)
    }
}

/// The unix permission bits of `meta`.
#[cfg(unix)]
pub(crate) fn mode(meta: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

/// Off unix there are no permission bits, only the read-only attribute: a
/// writable entry reads as 0o755 (directory) or 0o644 (file), a read-only one
/// as the same without the write bits, so freezing and fingerprinting see the
/// one change they care about.
#[cfg(not(unix))]
pub(crate) fn mode(meta: &Metadata) -> u32 {
    let full = if meta.is_dir() { 0o755 } else { 0o644 };
    if meta.permissions().readonly() {
        full & !0o222
    } else {
        full
    }
}

/// Sets the unix permission bits of `path` to `mode`.
#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// Off unix only the owner-write bit carries over: no write bit makes the
/// entry read-only (`Permissions::set_readonly`), any write bit clears it.
#[cfg(not(unix))]
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(mode & 0o222 == 0);
    fs::set_permissions(path, perms)
}

fn file_type(meta: &Metadata) -> FileType {
    meta.file_type()
}

pub(crate) fn freeze_tree(
    root: &TreeRoot,
) -> Result<Box<dyn FnOnce() -> Result<()> + Send + 'static>> {
    let mut entries = Vec::new();
    let walk = Walk::of(root)?;
    for (name, info) in &walk.entries {
        let path = root.join(name);
        let ft = file_type(info);
        if ft.is_symlink() {
            let target = fs::read_link(&path)
                .map_err(|_| violation(&format!("scored tree has an escaping symlink: {name}")))?;
            if escaping_symlink(root, name, &target) {
                return Err(violation(&format!(
                    "scored tree has an escaping symlink: {name}"
                )));
            }
            if let Err(e) = fs::metadata(&path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                return Err(violation(&format!(
                    "scored tree has an unresolvable symlink: {name}"
                )));
            }
            continue;
        }
        if !info.is_dir() && (!info.is_file() || walk.escaping_link(info)) {
            return Err(violation(&format!(
                "scored tree has a special file or shared hardlink: {name}"
            )));
        }
        entries.push(TreeEntry {
            name: name.clone(),
            mode: mode(info),
        });
    }
    let restore_root = root.root.clone();
    let restore_entries = entries.clone();
    let restore = move || {
        let mut first: Option<anyhow::Error> = None;
        for entry in restore_entries {
            let path = if entry.name == "." {
                restore_root.clone()
            } else {
                restore_root.join(&entry.name)
            };
            let Ok(info) = fs::symlink_metadata(&path) else {
                continue;
            };
            if info.file_type().is_symlink() {
                continue;
            }
            if let Err(e) = set_mode(&path, entry.mode & 0o777) {
                first.get_or_insert_with(|| e.into());
            }
        }
        if let Some(e) = first { Err(e) } else { Ok(()) }
    };
    for entry in entries.iter().rev() {
        let path = root.join(&entry.name);
        if let Err(e) = set_mode(&path, entry.mode & !0o222) {
            let _ = restore();
            return Err(e.into());
        }
    }
    Ok(Box::new(restore))
}

pub(crate) fn tree_fingerprint(root: &TreeRoot) -> Result<String> {
    let mut hasher = DefaultHasher::new();
    let walk = Walk::of(root)?;
    for (name, info) in &walk.entries {
        let path = root.join(name);
        name.hash(&mut hasher);
        mode(info).hash(&mut hasher);
        if info.is_file() {
            if walk.escaping_link(info) {
                return Err(violation(&format!(
                    "scorer created a shared hardlink: {name}"
                )));
            }
            let mut f = fs::File::open(path)?;
            let mut buf = [0u8; 8192];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                buf[..n].hash(&mut hasher);
            }
        } else if info.file_type().is_symlink() {
            fs::read_link(path)?.to_string_lossy().hash(&mut hasher);
        } else if !info.is_dir() {
            return Err(violation(&format!(
                "scored tree contains a special file: {name}"
            )));
        }
    }
    Ok(format!("{:016x}", hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_mode(path: &Path) -> u32 {
        mode(&fs::symlink_metadata(path).unwrap()) & 0o777
    }

    #[test]
    fn hardlinks_kept_inside_the_tree_are_frozen_and_restored() {
        let dir = crate::test_helpers::temp_path();
        let tree = dir.join("tree");
        fs::create_dir_all(tree.join("target/debug/deps")).unwrap();
        let uplifted = tree.join("target/debug/libpulse.rmeta");
        let built = tree.join("target/debug/deps/libpulse-1e10e1dd.rmeta");
        fs::write(&built, b"meta").unwrap();
        fs::hard_link(&built, &uplifted).unwrap();
        set_mode(&built, 0o644).unwrap();

        let root = TreeRoot::open(&tree).unwrap();
        let restore = freeze_tree(&root).unwrap();
        assert_eq!(file_mode(&built) & 0o222, 0, "frozen");
        assert_eq!(
            file_mode(&uplifted) & 0o222,
            0,
            "frozen through its other name"
        );
        let before = tree_fingerprint(&root).unwrap();
        assert_eq!(before, tree_fingerprint(&root).unwrap());
        restore().unwrap();
        assert_eq!(file_mode(&built), 0o644, "restored");
    }

    // Link counts and dev/ino identity are unix-only (`link_key`); off unix
    // an escaping hardlink is not detected yet (docs/windows-port.md).
    #[cfg(unix)]
    #[test]
    fn a_hardlink_leaving_the_tree_is_a_violation() {
        let dir = crate::test_helpers::temp_path();
        let tree = dir.join("tree");
        fs::create_dir_all(&tree).unwrap();
        let inside = tree.join("shared");
        fs::write(&inside, b"x").unwrap();
        fs::hard_link(&inside, dir.join("outside")).unwrap();

        let root = TreeRoot::open(&tree).unwrap();
        let Err(err) = freeze_tree(&root) else {
            panic!("freeze refuses");
        };
        let reason = err
            .downcast_ref::<crate::clones::ViolationError>()
            .expect("a violation")
            .reason
            .clone();
        assert_eq!(
            reason,
            "scored tree has a special file or shared hardlink: shared"
        );
        assert_eq!(file_mode(&inside) & 0o222, 0o200, "nothing was frozen");
        let err = tree_fingerprint(&root).unwrap_err();
        assert!(err.to_string().contains("shared hardlink: shared"), "{err}");
    }
}
