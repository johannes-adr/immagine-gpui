use std::path::PathBuf;

/// `IMMAGINE_DATA_DIR`, else the platform's per-user data directory.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("IMMAGINE_DATA_DIR") {
        return dir.into();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(any(target_os = "macos", target_os = "ios")) {
        home.join("Library/Application Support/immagine")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("immagine")
    }
}

fn file_name(raw: &str) -> String {
    raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

pub fn blob_path(db: &str, key: &str) -> PathBuf {
    data_dir().join(format!("{}-{}.bin", file_name(db), file_name(key)))
}

/// Removes every blob file of `db`.
pub fn blob_clear(db: &str) {
    let prefix = format!("{}-", file_name(db));
    if let Ok(entries) = std::fs::read_dir(data_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(".bin") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Total size of the blob files of `db`.
pub fn blob_size(db: &str) -> u64 {
    let prefix = format!("{}-", file_name(db));
    let Ok(entries) = std::fs::read_dir(data_dir()) else { return 0 };
    entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix) && name.ends_with(".bin")
        })
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Keys of the blob files of `db` (file names are the keys with unusual characters replaced, which ids do not have).
pub fn blob_keys(db: &str) -> Vec<String> {
    let prefix = format!("{}-", file_name(db));
    let Ok(entries) = std::fs::read_dir(data_dir()) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix(&prefix)?.strip_suffix(".bin").map(str::to_string)
        })
        .collect()
}

/// Bytes of every file under the data directory (blobs and settings).
pub fn data_size() -> u64 {
    fn walk(dir: &std::path::Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
        entries
            .flatten()
            .map(|e| match e.metadata() {
                Ok(m) if m.is_dir() => walk(&e.path()),
                Ok(m) => m.len(),
                Err(_) => 0,
            })
            .sum()
    }
    walk(&data_dir())
}

fn setting_path(key: &str) -> PathBuf {
    data_dir().join("settings").join(file_name(key))
}

pub fn setting_get(key: &str) -> Option<String> {
    std::fs::read_to_string(setting_path(key)).ok().filter(|v| !v.is_empty())
}

pub fn setting_set(key: &str, value: &str) {
    let path = setting_path(key);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, value);
}

pub fn setting_remove(key: &str) {
    let _ = std::fs::remove_file(setting_path(key));
}
