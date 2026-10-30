use std::path::PathBuf;

/// `IMMAGINE_DATA_DIR`, else the platform's per-user data directory.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("IMMAGINE_DATA_DIR") {
        return dir.into();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
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
