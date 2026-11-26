//! Data-driven categorization (PROJECT_BRIEF): rules are embedded TOML,
//! evaluated as path rules, then extension rules, then size heuristics.
//! Categories: Code, System, Archives, Media, Documents, Apps, Other.
//! Safety tiers: Safe, Review, Protected — any path component in
//! `protected.names` always forces Protected (never-deletable).

use std::collections::HashMap;

use serde::Deserialize;

use crate::tree::Tree;

pub const DEFAULT_RULES: &str = include_str!("rules.toml");

/// Stable category ids used on the wire (view/orbit formats, u8).
pub mod category {
    pub const CODE: u8 = 0;
    pub const SYSTEM: u8 = 1;
    pub const ARCHIVES: u8 = 2;
    pub const MEDIA: u8 = 3;
    pub const DOCUMENTS: u8 = 4;
    pub const APPS: u8 = 5;
    pub const OTHER: u8 = 6;
}

/// Stable tier ids used on the wire (u8).
pub mod tier {
    pub const SAFE: u8 = 0;
    pub const REVIEW: u8 = 1;
    pub const PROTECTED: u8 = 2;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Code,
    System,
    Archives,
    Media,
    Documents,
    Apps,
    Other,
}

impl Category {
    pub fn id(self) -> u8 {
        match self {
            Category::Code => category::CODE,
            Category::System => category::SYSTEM,
            Category::Archives => category::ARCHIVES,
            Category::Media => category::MEDIA,
            Category::Documents => category::DOCUMENTS,
            Category::Apps => category::APPS,
            Category::Other => category::OTHER,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Safe,
    Review,
    Protected,
}

impl Tier {
    pub fn id(self) -> u8 {
        match self {
            Tier::Safe => tier::SAFE,
            Tier::Review => tier::REVIEW,
            Tier::Protected => tier::PROTECTED,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathRule {
    pub category: Category,
    /// Matches when ANY path component equals this (case-insensitive).
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtRule {
    pub category: Category,
    /// File extension after the last dot, lower-case in the rule file.
    pub ext: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SizeRule {
    pub category: Category,
    /// Inclusive minimum logical size in MiB.
    pub min_size_mb: u64,
    /// When true, only applies to files without an extension.
    #[serde(default)]
    pub no_ext_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtectedRules {
    /// Any path component equal to one of these (case-insensitive) is
    /// Protected: pagefile.sys, hiberfil.sys, System Volume Information,
    /// the Windows folder, ... (PROJECT_BRIEF).
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rules {
    pub version: u8,
    #[serde(default)]
    pub path_rules: Vec<PathRule>,
    #[serde(default)]
    pub ext_rules: Vec<ExtRule>,
    #[serde(default)]
    pub size_rules: Vec<SizeRule>,
    pub protected: ProtectedRules,
    /// Default tier per category name (lowercase), overridden by protected.
    #[serde(default)]
    pub tier_by_category: HashMap<String, Tier>,
}

impl Rules {
    pub fn from_toml(s: &str) -> Result<Rules, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn default_rules() -> Rules {
        Rules::from_toml(DEFAULT_RULES).expect("embedded rules.toml must parse")
    }
}

/// Per-node category + tier, indexed by node id.
#[derive(Debug, Clone)]
pub struct CategoryMap {
    pub cat: Vec<u8>,
    pub tier: Vec<u8>,
}

/// Categorize every node in the tree (post-scan; never during the walk).
pub fn categorize(tree: &Tree, rules: &Rules) -> CategoryMap {
    let n = tree.len();
    let mut cat = vec![category::OTHER; n];
    let mut tier = vec![tier::REVIEW; n];

    for id in 0..n as u32 {
        let components = path_components(tree, id);
        let is_dir = tree.node(id).flags() & crate::tree::flags::DIR != 0;
        let logical = tree.node(id).agg_logical();

        // 1. path rules (first match wins)
        let mut c = category::OTHER;
        for rule in &rules.path_rules {
            if components
                .iter()
                .any(|comp| comp.eq_ignore_ascii_case(&rule.name))
            {
                c = rule.category.id();
                break;
            }
        }

        // 2. extension rules (files only)
        if c == category::OTHER && !is_dir {
            if let Some(ext) = file_ext(tree.name(id)) {
                for rule in &rules.ext_rules {
                    if ext == rule.ext {
                        c = rule.category.id();
                        break;
                    }
                }
            }
        }

        // 3. size heuristics (files only, still uncategorized)
        if c == category::OTHER && !is_dir {
            for rule in &rules.size_rules {
                let no_ext = file_ext(tree.name(id)).is_none();
                if rule.no_ext_only && !no_ext {
                    continue;
                }
                if logical >= rule.min_size_mb.saturating_mul(1024 * 1024) {
                    c = rule.category.id();
                    break;
                }
            }
        }

        // tier: protected names override the category default
        let mut t = rules
            .tier_by_category
            .get(category_name(c))
            .copied()
            .unwrap_or(Tier::Review);
        if components.iter().any(|comp| {
            rules
                .protected
                .names
                .iter()
                .any(|p| comp.eq_ignore_ascii_case(p))
        }) {
            t = Tier::Protected;
        }

        cat[id as usize] = c;
        tier[id as usize] = t.id();
    }

    CategoryMap { cat, tier }
}

/// Names from root to `id` (inclusive), root first.
fn path_components(tree: &Tree, id: u32) -> Vec<String> {
    let mut chain = Vec::new();
    let mut cur = Some(id);
    while let Some(x) = cur {
        chain.push(tree.name(x).to_string());
        cur = tree.parent(x);
    }
    chain.reverse();
    chain
}

fn file_ext(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 == name.len() {
        return None; // dotfiles like ".gitignore" have no extension
    }
    Some(name[dot + 1..].to_ascii_lowercase())
}

/// Human-readable name for a wire category id; unknown ids read as "other".
pub fn category_name(id: u8) -> &'static str {
    match id {
        category::CODE => "code",
        category::SYSTEM => "system",
        category::ARCHIVES => "archives",
        category::MEDIA => "media",
        category::DOCUMENTS => "documents",
        category::APPS => "apps",
        _ => "other",
    }
}

/// Human-readable name for a wire safety-tier id. Unknown ids are NOT
/// mapped to "review": an id outside the allowlist is reported as "unknown"
/// so a CSV never implies a tier the app did not assign.
pub fn tier_name(id: u8) -> &'static str {
    match id {
        tier::SAFE => "safe",
        tier::REVIEW => "review",
        tier::PROTECTED => "protected",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rollup::recompute_rollups;
    use crate::tree::flags;

    fn tree_with() -> Tree {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        let win = t.insert_child(root, "Windows", flags::DIR, 0, 0, 0);
        t.insert_child(win, "explorer.exe", 0, 0, 4_000_000, 4_000_000);
        let node_modules = t.insert_child(root, "node_modules", flags::DIR, 0, 0, 0);
        t.insert_child(node_modules, "index.js", 0, 0, 12_000, 12_000);
        let movies = t.insert_child(root, "Movies", flags::DIR, 0, 0, 0);
        t.insert_child(movies, "film.mp4", 0, 0, 800_000_000, 800_000_000);
        t.insert_child(root, "notes.pdf", 0, 0, 2_000_000, 2_000_000);
        t.insert_child(root, "pagefile.sys", 0, 0, 8_000_000_000, 8_000_000_000);
        t.insert_child(root, "mystery", 0, 0, 600 * 1024 * 1024, 600 * 1024 * 1024);
        t.insert_child(root, "small.dat", 0, 0, 5_000_000, 5_000_000);
        t.insert_child(root, "setup.exe", 0, 0, 50_000_000, 50_000_000);
        recompute_rollups(&mut t);
        t
    }

    // insert order: root=0, Windows=1, explorer=2, node_modules=3,
    // index.js=4, Movies=5, film.mp4=6, notes.pdf=7, pagefile.sys=8,
    // mystery=9, small.dat=10, setup.exe=11

    #[test]
    fn default_rules_parse() {
        let rules = Rules::default_rules();
        assert_eq!(rules.version, 1);
        assert!(!rules.path_rules.is_empty());
        assert!(!rules.ext_rules.is_empty());
        assert!(rules.protected.names.iter().any(|n| n == "pagefile.sys"));
    }

    #[test]
    fn path_rules_apply_to_children() {
        let t = tree_with();
        let rules = Rules::default_rules();
        let m = categorize(&t, &rules);
        assert_eq!(m.cat[3], category::CODE); // node_modules dir
        assert_eq!(m.cat[4], category::CODE); // index.js inside it
    }

    #[test]
    fn ext_rules_classify_files() {
        let t = tree_with();
        let rules = Rules::default_rules();
        let m = categorize(&t, &rules);
        assert_eq!(m.cat[6], category::MEDIA); // film.mp4
        assert_eq!(m.cat[7], category::DOCUMENTS); // notes.pdf
                                                   // explorer.exe lives inside "Windows": path rules run FIRST, so the
                                                   // System path rule wins over the .exe extension rule.
        assert_eq!(m.cat[2], category::SYSTEM);
        // A bare exe outside any matched path is Apps.
        assert_eq!(m.cat[11], category::APPS); // setup.exe at root
    }

    #[test]
    fn size_heuristic_no_ext_only() {
        let t = tree_with();
        let rules = Rules::default_rules();
        let m = categorize(&t, &rules);
        assert_eq!(m.cat[9], category::OTHER); // mystery: 600 MB, no ext -> other rule
                                               // small.dat has an extension -> size rule (no_ext_only) skipped.
        assert_eq!(m.cat[10], category::OTHER);
    }

    #[test]
    fn protected_tier_overrides_category_tier() {
        let t = tree_with();
        let rules = Rules::default_rules();
        let m = categorize(&t, &rules);
        assert_eq!(m.tier[8], tier::PROTECTED); // pagefile.sys
        assert_eq!(m.tier[1], tier::PROTECTED); // Windows folder
        assert_eq!(m.tier[2], tier::PROTECTED); // explorer.exe inside Windows
                                                // Normal items follow tier_by_category.
        assert_eq!(m.tier[6], tier::SAFE); // media -> safe
        assert_eq!(m.tier[7], tier::REVIEW); // documents -> review
    }

    #[test]
    fn dotfiles_have_no_extension() {
        assert_eq!(file_ext(".gitignore"), None);
        assert_eq!(file_ext("archive.tar.gz"), Some("gz".to_string()));
        assert_eq!(file_ext("readme"), None);
    }
}
