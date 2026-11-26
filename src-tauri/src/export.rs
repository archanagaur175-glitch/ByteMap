//! CSV export (P8.T4): the open folder's items or the cleanup basket as
//! RFC 4180 CSV (comma separated, CRLF line ends, minimal quoting, UTF-8).
//!
//! No dialog plugin: the file is written to `%LOCALAPPDATA%\ByteMap\exports`
//! (`BYTEMAP_EXPORT_DIR` overrides it for manual tests) and the command
//! returns the absolute path so the UI can show it. That keeps the app
//! local-only and the whole feature testable without a webview.
//!
//! "Current view" means every DIRECT child of the open node, sorted by
//! aggregate size descending â€” the same order the view shows, but without
//! the folded "N small items" pseudo-row (a folded sum has no single path).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bytemap_actions::Basket;
use bytemap_core::categorizer::{category_name, tier_name, CategoryMap};
use bytemap_core::rollup::children_by_size;
use bytemap_core::tree::flags;
use bytemap_core::{NodeId, Tree};
use serde_json::json;
use tauri::State;

use crate::pathres::node_path;
use crate::state::SharedSession;

/// File extension of an export.
pub const EXPORT_EXT: &str = "csv";

/// Hard cap on one export. A folder can hold millions of children; a CSV
/// that big is not useful and would block the IPC thread. Rows beyond the
/// cap are dropped and the result says `truncated: true` (never silent).
pub const MAX_EXPORT_ROWS: usize = 100_000;

const HEADER: &str = "path,name,kind,logical_bytes,on_disk_bytes,category,tier";

/// One CSV row, before quoting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub path: String,
    pub name: String,
    /// "folder", "file", or "" when the kind cannot be known (basket item
    /// whose node id no longer belongs to the tree).
    pub kind: &'static str,
    pub logical: u64,
    pub on_disk: u64,
    pub category: &'static str,
    pub tier: &'static str,
}

/// Export folder: `%LOCALAPPDATA%\ByteMap\exports` (`BYTEMAP_EXPORT_DIR`
/// overrides it for manual tests). The app is local-only; nothing leaves
/// the machine.
pub fn export_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("BYTEMAP_EXPORT_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("ByteMap").join("exports")
}

/// Rows for the open node's direct children, size descending.
pub fn view_rows(tree: &Tree, cats: &CategoryMap, root: &Path, id: NodeId) -> Vec<Row> {
    let mut rows = Vec::new();
    for child in children_by_size(tree, id) {
        let Some(path) = node_path(tree, root, child) else {
            continue; // not anchored to this root: nothing truthful to write
        };
        let node = tree.node(child);
        let cat = cats.cat.get(child as usize).copied().unwrap_or(0);
        let tier = cats.tier.get(child as usize).copied().unwrap_or(0);
        rows.push(Row {
            path: path.to_string_lossy().into_owned(),
            name: tree.name(child).to_string(),
            kind: if node.flags() & flags::DIR != 0 {
                "folder"
            } else {
                "file"
            },
            logical: node.agg_logical(),
            on_disk: node.agg_on_disk(),
            category: category_name(cat),
            tier: tier_name(tier),
        });
    }
    rows
}

/// Rows for the staged cleanup basket, in staging order. The kind comes from
/// the current tree; a stale id leaves it empty rather than guessing.
pub fn basket_rows(tree: &Tree, basket: &Basket) -> Vec<Row> {
    basket
        .items()
        .iter()
        .map(|item| {
            let kind = if (item.node_id as usize) < tree.len() {
                if tree.node(item.node_id).flags() & flags::DIR != 0 {
                    "folder"
                } else {
                    "file"
                }
            } else {
                ""
            };
            Row {
                path: item.path.clone(),
                name: item.name.clone(),
                kind,
                logical: item.logical,
                on_disk: item.on_disk,
                category: category_name(item.cat),
                tier: tier_name(item.tier),
            }
        })
        .collect()
}

/// Keep the first `max` rows and report whether anything was dropped.
fn cap(rows: Vec<Row>, max: usize) -> (Vec<Row>, bool) {
    if rows.len() <= max {
        return (rows, false);
    }
    let truncated = rows.into_iter().take(max).collect();
    (truncated, true)
}

/// Render rows as CSV: header first, CRLF line ends, a field is quoted only
/// when it contains a comma, a quote or a line break (quotes are doubled).
pub fn render_csv(rows: &[Row]) -> String {
    let mut out = String::with_capacity(HEADER.len() + 2 + rows.len() * 96);
    out.push_str(HEADER);
    out.push_str("\r\n");
    for row in rows {
        push_field(&mut out, &row.path);
        out.push(',');
        push_field(&mut out, &row.name);
        out.push(',');
        out.push_str(row.kind);
        out.push(',');
        out.push_str(&row.logical.to_string());
        out.push(',');
        out.push_str(&row.on_disk.to_string());
        out.push(',');
        out.push_str(row.category);
        out.push(',');
        out.push_str(row.tier);
        out.push_str("\r\n");
    }
    out
}

fn push_field(out: &mut String, value: &str) {
    let needs_quotes = value
        .chars()
        .any(|c| c == ',' || c == '"' || c == '\r' || c == '\n');
    if !needs_quotes {
        out.push_str(value);
        return;
    }
    out.push('"');
    for c in value.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
}

/// Pick the next free `bytemap-<scope>-<created>[-n].csv` path.
fn unique_path(dir: &Path, scope: &str, created_unix: u64) -> PathBuf {
    let base = format!("bytemap-{scope}-{created_unix}");
    for n in 1..=9999 {
        let name = if n == 1 {
            format!("{base}.{EXPORT_EXT}")
        } else {
            format!("{base}-{n}.{EXPORT_EXT}")
        };
        let path = dir.join(&name);
        if !path.exists() {
            return path;
        }
    }
    dir.join(format!("{base}-{}.{EXPORT_EXT}", 10_000))
}

/// Write `csv` into `dir` under a fresh name; returns the file name + path.
pub fn write_export(dir: &Path, scope: &str, csv: &str) -> Result<(String, PathBuf), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = unique_path(dir, scope, created);
    fs::write(&path, csv).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((name, path))
}

/// Export the current view or the basket. Returns where the file landed.
#[tauri::command]
pub fn export_csv(
    scope: String,
    node_id: Option<u32>,
    session: State<'_, SharedSession>,
) -> Result<serde_json::Value, String> {
    export_in(&export_dir(), &scope, node_id, session.inner())
}

/// Testable core behind `export_csv`.
pub fn export_in(
    dir: &Path,
    scope: &str,
    node_id: Option<u32>,
    shared: &SharedSession,
) -> Result<serde_json::Value, String> {
    // Build the rows under the lock, then render + write outside it: a large
    // export must not block a scan's progress or a view request.
    let (rows, total, truncated) = {
        let s = shared.lock().unwrap();
        if s.running {
            return Err("a scan is running; finish or cancel it before exporting".to_string());
        }
        let root = s
            .tree
            .root()
            .ok_or_else(|| "scan a folder or load a snapshot first".to_string())?;
        match scope {
            "view" => {
                let id = node_id.unwrap_or(root);
                if id as usize >= s.tree.len() {
                    return Err(
                        "that node id is not part of the current tree; reopen the folder"
                            .to_string(),
                    );
                }
                let rows = view_rows(&s.tree, &s.cats, &s.root_path, id);
                let total = rows.len();
                let (rows, truncated) = cap(rows, MAX_EXPORT_ROWS);
                (rows, total, truncated)
            }
            "basket" => {
                let rows = basket_rows(&s.tree, &s.basket);
                let total = rows.len();
                let (rows, truncated) = cap(rows, MAX_EXPORT_ROWS);
                (rows, total, truncated)
            }
            other => return Err(format!("unknown export scope {other:?}")),
        }
    };

    let csv = render_csv(&rows);
    let (name, path) = write_export(dir, scope, &csv)?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "name": name,
        "scope": scope,
        "rows": rows.len(),
        "totalRows": total,
        "bytes": csv.len(),
        "truncated": truncated,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    use bytemap_actions::{Basket, BasketItem};
    use bytemap_core::categorizer::{categorize, CategoryMap, Rules};
    use bytemap_core::recompute_rollups;

    use super::*;
    use crate::state::Session;

    /// Per-test temp folder: tests run in parallel and must not share one.
    fn temp_dir(tag: &str) -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("bytemap-export-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tree_and_cats() -> (Tree, CategoryMap) {
        let mut tree = Tree::new();
        let root = tree.insert_root(r"C:\fixture");
        let docs = tree.insert_child(root, "docs", flags::DIR, 0, 0, 0);
        tree.insert_child(docs, "a.pdf", 0, 0, 10, 4096);
        tree.insert_child(root, "movie.mp4", 0, 0, 500, 4096);
        tree.insert_child(root, "tiny, \"odd\".txt", 0, 0, 5, 4096);
        recompute_rollups(&mut tree);
        let cats = categorize(&tree, &Rules::default_rules());
        (tree, cats)
    }

    fn shared(tree: Tree, cats: CategoryMap, basket: Basket) -> SharedSession {
        Arc::new(Mutex::new(Session {
            tree,
            cats,
            root_path: PathBuf::from(r"C:\fixture"),
            basket,
            running: false,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            scan_tier: Some(bytemap_core::ScanTier::Walker),
            scan_ms: 7,
            loaded_snapshot: None,
            usn_checkpoint: None,
        }))
    }

    #[test]
    fn fields_are_quoted_only_when_needed_and_use_crlf() {
        let rows = vec![Row {
            path: r#"C:\fixture\tiny, "odd".txt"#.to_string(),
            name: "tiny, \"odd\".txt".to_string(),
            kind: "file",
            logical: 5,
            on_disk: 4096,
            category: "documents",
            tier: "safe",
        }];
        let csv = render_csv(&rows);
        assert!(csv.starts_with("path,name,kind,logical_bytes,on_disk_bytes,category,tier\r\n"));
        assert!(csv.ends_with("\r\n"), "every record ends with CRLF");
        assert!(csv.contains("\"C:\\fixture\\tiny, \"\"odd\"\".txt\",\"tiny, \"\"odd\"\".txt\",file,5,4096,documents,safe\r\n"));
        assert_eq!(csv.matches('\n').count(), 2, "header + one row");
    }

    #[test]
    fn a_plain_field_is_not_quoted() {
        let mut out = String::new();
        push_field(&mut out, "plain name.txt");
        assert_eq!(out, "plain name.txt");
        let mut out = String::new();
        push_field(&mut out, "line\nbreak");
        assert_eq!(out, "\"line\nbreak\"");
    }

    #[test]
    fn view_rows_are_size_sorted_with_paths_and_kinds() {
        let (tree, cats) = tree_and_cats();
        let rows = view_rows(&tree, &cats, Path::new(r"C:\fixture"), 0);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "movie.mp4");
        assert_eq!(rows[0].kind, "file");
        assert_eq!(rows[0].path, r"C:\fixture\movie.mp4");
        assert_eq!(rows[1].name, "docs");
        assert_eq!(rows[1].kind, "folder");
        assert_eq!(rows[1].path, r"C:\fixture\docs");
        assert!(!rows[1].category.is_empty() && !rows[1].tier.is_empty());
        assert_eq!(rows[2].name, "tiny, \"odd\".txt");
    }

    #[test]
    fn basket_rows_keep_staging_order_and_learn_kind_from_the_tree() {
        let (tree, _cats) = tree_and_cats();
        let mut basket = Basket::new();
        basket
            .add(BasketItem {
                node_id: 1, // docs (folder)
                path: r"C:\fixture\docs".to_string(),
                name: "docs".to_string(),
                logical: 10,
                on_disk: 4096,
                mtime: 0,
                tier: bytemap_core::categorizer::tier::SAFE,
                cat: bytemap_core::categorizer::category::DOCUMENTS,
            })
            .unwrap();
        basket
            .add(BasketItem {
                node_id: 99, // stale id: kind unknown, never guessed
                path: r"C:\gone.tmp".to_string(),
                name: "gone.tmp".to_string(),
                logical: 1,
                on_disk: 1,
                mtime: 0,
                tier: bytemap_core::categorizer::tier::SAFE,
                cat: 0,
            })
            .unwrap();
        let rows = basket_rows(&tree, &basket);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "folder");
        assert_eq!(rows[0].category, "documents");
        assert_eq!(rows[0].tier, "safe");
        assert_eq!(rows[1].kind, "");
        assert_eq!(rows[1].path, r"C:\gone.tmp");
    }

    #[test]
    fn cap_reports_truncation() {
        let row = Row {
            path: "p".to_string(),
            name: "n".to_string(),
            kind: "file",
            logical: 0,
            on_disk: 0,
            category: "other",
            tier: "safe",
        };
        let (kept, truncated) = cap(vec![row.clone(), row.clone(), row], 2);
        assert_eq!(kept.len(), 2);
        assert!(truncated);
        let (kept, truncated) = cap(vec![], 2);
        assert!(kept.is_empty() && !truncated);
    }

    #[test]
    fn export_in_writes_a_new_file_each_time_and_returns_its_path() {
        let (tree, cats) = tree_and_cats();
        let session = shared(tree, cats, Basket::new());
        let dir = temp_dir("view");

        let first = export_in(&dir, "view", Some(0), &session).unwrap();
        let second = export_in(&dir, "view", Some(0), &session).unwrap();
        let first_path = first["path"].as_str().unwrap();
        let second_path = second["path"].as_str().unwrap();
        assert_ne!(first_path, second_path, "never overwrite an export");
        assert!(first_path.ends_with(".csv"));
        assert_eq!(first["rows"], 3);
        assert_eq!(first["totalRows"], 3);
        assert_eq!(first["truncated"], false);

        let text = fs::read_to_string(first_path).unwrap();
        let mut lines = text.split("\r\n");
        assert_eq!(
            lines.next().unwrap(),
            "path,name,kind,logical_bytes,on_disk_bytes,category,tier"
        );
        assert!(lines.next().unwrap().starts_with(r"C:\fixture\movie.mp4,"));
        assert_eq!(text.matches("\r\n").count(), 4, "header + 3 rows");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_in_refuses_unknown_scopes_running_scans_and_empty_sessions() {
        let (tree, cats) = tree_and_cats();
        let dir = temp_dir("refuse");
        let session = shared(tree, cats, Basket::new());

        let err = export_in(&dir, "everything", None, &session).unwrap_err();
        assert!(err.contains("unknown export scope"), "{err}");

        let err = export_in(&dir, "view", Some(9_999), &session).unwrap_err();
        assert!(err.contains("not part of the current tree"), "{err}");

        session.lock().unwrap().running = true;
        let err = export_in(&dir, "view", None, &session).unwrap_err();
        assert!(err.contains("scan is running"), "{err}");
        session.lock().unwrap().running = false;

        let empty = Session {
            tree: Tree::new(),
            cats: CategoryMap {
                cat: Vec::new(),
                tier: Vec::new(),
            },
            root_path: PathBuf::from(r"C:\empty"),
            basket: Basket::new(),
            running: false,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            scan_tier: None,
            scan_ms: 0,
            loaded_snapshot: None,
            usn_checkpoint: None,
        };
        let empty: SharedSession = Arc::new(Mutex::new(empty));
        let err = export_in(&dir, "view", None, &empty).unwrap_err();
        assert!(err.contains("scan a folder"), "{err}");

        let _ = fs::remove_dir_all(&dir);
    }
}
