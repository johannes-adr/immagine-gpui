//! Album folders, encoded in album names: "Festival/Elf 2026" is the album "Elf 2026" inside
//! the folder "Festival". Immich has only flat albums; the name is the one place that survives
//! every client and sync.
//!
//! Port of `Immagine/Models/AlbumTree.swift` (`AlbumPath`, `AlbumFolder`, `AlbumTree`).
//! Spec: `ModelTests.swift` (album folder tests). The FavoritesTests sort-bar cases use the
//! UI `SortTarget` and are not ported here.
//!
//! Folder and album ordering follow `localizedStandardCompare` as closely as needed: case-insensitive,
//! digit runs compared by number ("Album 2" before "Album 10").

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::album::Album;

/// Helpers for "/"-separated album paths. Static functions, as the Swift `enum AlbumPath`.
pub struct AlbumPath;

impl AlbumPath {
    pub const SEPARATOR: &'static str = "/";

    /// Trimmed, non-empty components: " A / B/ C " gives `["A", "B", "C"]`.
    pub fn components(name: &str) -> Vec<&str> {
        name.split('/').map(str::trim).filter(|s| !s.is_empty()).collect()
    }

    pub fn normalize(path: &str) -> String {
        Self::components(path).join(Self::SEPARATOR)
    }

    /// Folder of an album name ("" = top level).
    pub fn folder_of_album_name(name: &str) -> String {
        let parts = Self::components(name);
        parts[..parts.len().saturating_sub(1)].join(Self::SEPARATOR)
    }

    /// What the tile shows. Falls back to the raw name for degenerate names such as "/".
    pub fn leaf_of_album_name(name: &str) -> String {
        Self::components(name).last().map_or_else(|| name.to_string(), |s| s.to_string())
    }

    pub fn join(folder: &str, leaf: &str) -> String {
        if folder.is_empty() {
            Self::normalize(leaf)
        } else {
            Self::normalize(&format!("{folder}{}{leaf}", Self::SEPARATOR))
        }
    }

    pub fn parent_of(folder: &str) -> String {
        let parts = Self::components(folder);
        parts[..parts.len().saturating_sub(1)].join(Self::SEPARATOR)
    }

    pub fn name_of(folder: &str) -> String {
        Self::components(folder).last().map_or_else(|| folder.to_string(), |s| s.to_string())
    }

    /// `path` is `folder` itself or lies anywhere below it.
    pub fn is_within(path: &str, folder: &str) -> bool {
        if folder.is_empty() {
            return true;
        }
        let p = Self::normalize(path);
        p == folder || p.starts_with(&format!("{folder}{}", Self::SEPARATOR))
    }

    /// Re-roots a path that lies within `old` under `new` (rename or move).
    pub fn reparent(path: &str, from: &str, to: &str) -> String {
        let p = Self::normalize(path);
        if !Self::is_within(&p, from) {
            return p;
        }
        if from.is_empty() {
            return Self::join(to, &p);
        }
        // `from` is a normalized prefix of `p`, so slicing at its length is on a char boundary.
        let rest = &p[from.len()..];
        Self::normalize(&format!("{to}{}{rest}", Self::SEPARATOR))
    }
}

/// A folder as shown in a grid: path, display name and recursive content.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AlbumFolder {
    pub path: String,
    pub name: String,
    /// Albums at any depth below this folder.
    pub album_count: usize,
}

impl AlbumFolder {
    pub fn id(&self) -> &str {
        &self.path
    }
}

/// The folder hierarchy derived from album names, plus local folders that hold no album yet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AlbumTree {
    folders_by_parent: BTreeMap<String, Vec<AlbumFolder>>,
    albums_by_folder: BTreeMap<String, Vec<Album>>,
    all_folder_paths: BTreeSet<String>,
}

impl AlbumTree {
    pub fn new(albums: &[Album], local_folders: &[String]) -> Self {
        let mut folder_paths = BTreeSet::new();
        let mut by_folder: BTreeMap<String, Vec<Album>> = BTreeMap::new();
        for album in albums {
            let folder = AlbumPath::folder_of_album_name(&album.album_name);
            add_with_ancestors(&folder, &mut folder_paths);
            by_folder.entry(folder).or_default().push(album.clone());
        }
        for path in local_folders {
            add_with_ancestors(&AlbumPath::normalize(path), &mut folder_paths);
        }
        folder_paths.remove("");

        let mut recursive: BTreeMap<String, usize> = BTreeMap::new();
        for (folder, list) in by_folder.iter().filter(|(f, _)| !f.is_empty()) {
            let mut path = folder.clone();
            while !path.is_empty() {
                *recursive.entry(path.clone()).or_default() += list.len();
                path = AlbumPath::parent_of(&path);
            }
        }

        let mut by_parent: BTreeMap<String, Vec<AlbumFolder>> = BTreeMap::new();
        for path in &folder_paths {
            by_parent.entry(AlbumPath::parent_of(path)).or_default().push(AlbumFolder {
                path: path.clone(),
                name: AlbumPath::name_of(path),
                album_count: recursive.get(path).copied().unwrap_or(0),
            });
        }
        for list in by_parent.values_mut() {
            list.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        }
        for list in by_folder.values_mut() {
            list.sort_by(|a, b| {
                natural_cmp(
                    &AlbumPath::leaf_of_album_name(&a.album_name),
                    &AlbumPath::leaf_of_album_name(&b.album_name),
                )
            });
        }

        AlbumTree { folders_by_parent: by_parent, albums_by_folder: by_folder, all_folder_paths: folder_paths }
    }

    pub fn folders(&self, path: &str) -> &[AlbumFolder] {
        self.folders_by_parent.get(path).map_or(&[], Vec::as_slice)
    }

    pub fn albums(&self, path: &str) -> &[Album] {
        self.albums_by_folder.get(path).map_or(&[], Vec::as_slice)
    }

    pub fn contains_folder(&self, path: &str) -> bool {
        path.is_empty() || self.all_folder_paths.contains(path)
    }

    /// Every album at or below `path`.
    pub fn all_albums_under(&self, path: &str) -> Vec<&Album> {
        self.albums_by_folder
            .iter()
            .filter(|(folder, _)| AlbumPath::is_within(folder, path))
            .flat_map(|(_, list)| list.iter())
            .collect()
    }

    /// Every folder path at or below `path`, deepest first (safe delete order).
    pub fn all_folders_under(&self, path: &str) -> Vec<String> {
        let mut paths: Vec<String> = self
            .all_folder_paths
            .iter()
            .filter(|p| AlbumPath::is_within(p, path))
            .cloned()
            .collect();
        paths.sort_by_key(|p| std::cmp::Reverse(AlbumPath::components(p).len()));
        paths
    }

    /// Up to `limit` albums for a folder tile's 2x2 collage: direct albums first, then albums
    /// of subfolders.
    pub fn collage_albums(&self, path: &str, limit: usize) -> Vec<&Album> {
        let mut result: Vec<&Album> = self.albums(path).iter().take(limit).collect();
        if result.len() < limit {
            for folder in self.folders(path) {
                let need = limit - result.len();
                result.extend(self.collage_albums(&folder.path, need));
                if result.len() >= limit {
                    break;
                }
            }
        }
        result.truncate(limit);
        result
    }
}

fn add_with_ancestors(path: &str, set: &mut BTreeSet<String>) {
    let mut p = path.to_string();
    while !p.is_empty() && set.insert(p.clone()) {
        p = AlbumPath::parent_of(&p);
    }
}

/// Ascending order that treats digit runs as numbers and ignores case first, like
/// `localizedStandardCompare`. Ties fall back to plain string order so the result is total.
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => break,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_digits(&mut ai);
                let nb = take_digits(&mut bi);
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = fold(x).cmp(&fold(y));
                if ord != Ordering::Equal {
                    return ord;
                }
                ai.next();
                bi.next();
            }
        }
    }
    a.cmp(b)
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(id: &str, name: &str) -> Album {
        Album {
            id: id.into(),
            album_name: name.into(),
            description: None,
            album_thumbnail_asset_id: None,
            created_at: None,
            updated_at: None,
            asset_count: 1,
            shared: false,
            has_shared_link: false,
            order: None,
        }
    }

    fn ids<'a>(albums: impl IntoIterator<Item = &'a Album>) -> Vec<&'a str> {
        albums.into_iter().map(|a| a.id.as_str()).collect()
    }

    #[test]
    fn album_path_parsing() {
        assert_eq!(AlbumPath::components(" Festival / Elf 2026 "), vec!["Festival", "Elf 2026"]);
        assert_eq!(AlbumPath::folder_of_album_name("Reisen/2026/Italien"), "Reisen/2026");
        assert_eq!(AlbumPath::leaf_of_album_name("Reisen/2026/Italien"), "Italien");
        assert_eq!(AlbumPath::folder_of_album_name("Urlaub"), "");
        assert_eq!(AlbumPath::leaf_of_album_name("/"), "/", "degenerate names keep their raw text");
        assert_eq!(AlbumPath::leaf_of_album_name("A//B"), "B");
        assert_eq!(AlbumPath::join("", "Neu"), "Neu");
        assert_eq!(AlbumPath::join("Festival", " Tomorrowland "), "Festival/Tomorrowland");
        assert_eq!(AlbumPath::parent_of("A/B/C"), "A/B");
        assert_eq!(AlbumPath::name_of("A/B/C"), "C");
    }

    #[test]
    fn album_path_within_and_reparent() {
        assert!(AlbumPath::is_within("A/B", "A"));
        assert!(AlbumPath::is_within("A", "A"));
        assert!(!AlbumPath::is_within("AB/C", "A"), "prefix match must respect component boundaries");
        assert!(AlbumPath::is_within("anything", ""));
        assert_eq!(AlbumPath::reparent("A/B/C", "A", "X"), "X/B/C");
        assert_eq!(AlbumPath::reparent("A/B/C", "A/B", "A"), "A/C", "dissolving a folder moves content up");
        assert_eq!(AlbumPath::reparent("C", "", "X"), "X/C");
        assert_eq!(AlbumPath::reparent("A/C", "A", ""), "C");
        assert_eq!(AlbumPath::reparent("Other/C", "A", "X"), "Other/C", "paths outside stay untouched");
    }

    #[test]
    fn album_tree_structure() {
        let tree = AlbumTree::new(
            &[
                album("1", "Festival/Elf16"),
                album("2", "Festival/Tomorrowland 2026"),
                album("3", "Reisen/2026/Italien"),
                album("4", "Posteingang"),
                album("5", "festival/klein"),
            ],
            &["Leer".to_string(), "Reisen/Später".to_string()],
        );

        let top: Vec<String> = tree.folders("").iter().map(|f| f.path.clone()).collect();
        let mut sorted_top = top.clone();
        sorted_top.sort();
        assert_eq!(sorted_top, vec!["Festival", "Leer", "Reisen", "festival"], "folder names are case-sensitive");
        assert_eq!(top.last().map(String::as_str), Some("Reisen"));
        assert_eq!(ids(tree.albums("")), vec!["4"]);
        assert_eq!(ids(tree.albums("Festival")), vec!["1", "2"]);
        let reisen: Vec<&str> = tree.folders("Reisen").iter().map(|f| f.name.as_str()).collect();
        assert_eq!(reisen, vec!["2026", "Später"]);
        assert_eq!(tree.folders("").iter().find(|f| f.path == "Reisen").unwrap().album_count, 1, "counts are recursive");
        assert_eq!(tree.folders("").iter().find(|f| f.path == "Leer").unwrap().album_count, 0);
        assert!(tree.contains_folder("Reisen/2026"));
        assert!(!tree.contains_folder("Nope"));
        assert_eq!(ids(tree.all_albums_under("Reisen")), vec!["3"]);
        let under = tree.all_folders_under("Reisen");
        let under_set: BTreeSet<&str> = under.iter().map(String::as_str).collect();
        assert_eq!(under_set, BTreeSet::from(["Reisen", "Reisen/2026", "Reisen/Später"]));
        assert_eq!(under.last().map(String::as_str), Some("Reisen"), "deepest first, the folder itself last");
    }

    #[test]
    fn folder_collage_uses_nested_albums() {
        let tree = AlbumTree::new(
            &[
                album("1", "F/a"),
                album("2", "F/Sub/b"),
                album("3", "F/Sub/c"),
                album("4", "F/Sub/Deep/d"),
                album("5", "F/Sub/e"),
            ],
            &[],
        );
        assert_eq!(ids(tree.collage_albums("F", 4)), vec!["1", "2", "3", "5"]);
        assert_eq!(tree.collage_albums("F", 4).len(), 4);
        assert!(tree.collage_albums("Missing", 4).is_empty());
    }

    #[test]
    fn natural_order_numbers_and_case() {
        assert_eq!(natural_cmp("Album 2", "Album 10"), Ordering::Less);
        assert_eq!(natural_cmp("2026", "Später"), Ordering::Less);
        assert_eq!(natural_cmp("Berge", "Reisen"), Ordering::Less);
    }
}
