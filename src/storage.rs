//! On-disk layout, same shape as the Tauri build (`paths.rs` + `profiles.rs`):
//!
//! ```text
//! <local data>/<identifier>/profiles.json
//! <local data>/<identifier>/profiles/<active>/projects.json
//! ```
//!
//! The native build uses its own identifier so the two apps never write the same file at once.
//! On first launch it imports the Tauri build's `projects.json` (read-only copy).

use crate::model::{now_ms, ProjectsFile};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// `identifier` from the web build's `tauri.conf.json`.
pub const TAURI_IDENTIFIER: &str = "com.kc1t.alethe";
pub const NATIVE_IDENTIFIER: &str = "com.kc1t.alethe.native";
const DEFAULT_PROFILE_ID: &str = "default";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProfileMeta {
    pub id: String,
    pub name: String,
    pub created_at_ms: u64,
    pub last_used_at_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProfilesIndex {
    pub version: u32,
    pub active_profile_id: String,
    pub profiles: Vec<ProfileMeta>,
}

impl ProfilesIndex {
    fn default_index() -> Self {
        let now = now_ms();
        Self {
            version: 1,
            active_profile_id: DEFAULT_PROFILE_ID.into(),
            profiles: vec![ProfileMeta {
                id: DEFAULT_PROFILE_ID.into(),
                name: "Default".into(),
                created_at_ms: now,
                last_used_at_ms: now,
            }],
        }
    }

    pub fn active_name(&self) -> String {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_profile_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Default".into())
    }
}

pub fn home_dir() -> String {
    dirs_next::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".into())
}

fn root_for(identifier: &str) -> Option<PathBuf> {
    dirs_next::data_local_dir().map(|d| d.join(identifier))
}

fn read_index(root: &Path) -> Option<ProfilesIndex> {
    let raw = fs::read_to_string(root.join("profiles.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub struct Storage {
    pub root: PathBuf,
    pub index: ProfilesIndex,
    /// Where the projects were imported from on this launch, if they were.
    pub imported_from: Option<PathBuf>,
}

impl Storage {
    pub fn open() -> Self {
        // `ALETHE_NATIVE_DATA_DIR` points the app at another data root (tests, portable installs).
        let root = std::env::var_os("ALETHE_NATIVE_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| root_for(NATIVE_IDENTIFIER))
            .unwrap_or_else(|| PathBuf::from(".alethe-native"));
        let index = read_index(&root).unwrap_or_else(|| {
            let index = ProfilesIndex::default_index();
            let _ = atomic_write(
                &root.join("profiles.json"),
                &serde_json::to_string_pretty(&index).unwrap_or_default(),
            );
            index
        });
        let mut storage = Self {
            root,
            index,
            imported_from: None,
        };
        storage.import_from_tauri_build();
        storage
    }

    pub fn profile_dir(&self) -> PathBuf {
        self.root
            .join("profiles")
            .join(&self.index.active_profile_id)
    }

    pub fn projects_path(&self) -> PathBuf {
        self.profile_dir().join("projects.json")
    }

    /// Path of the Tauri build's `projects.json` for its active profile, when it exists.
    pub fn tauri_projects_path() -> Option<PathBuf> {
        let root = root_for(TAURI_IDENTIFIER)?;
        let profile = read_index(&root)
            .map(|i| i.active_profile_id)
            .unwrap_or_else(|| DEFAULT_PROFILE_ID.into());
        let path = root.join("profiles").join(profile).join("projects.json");
        path.is_file().then_some(path)
    }

    fn import_from_tauri_build(&mut self) {
        let target = self.projects_path();
        if target.exists() {
            return;
        }
        let Some(source) = Self::tauri_projects_path() else {
            return;
        };
        let Ok(raw) = fs::read_to_string(&source) else {
            return;
        };
        if serde_json::from_str::<ProjectsFile>(&raw).is_err() {
            return;
        }
        if atomic_write(&target, &raw).is_ok() {
            self.imported_from = Some(source);
        }
    }

    pub fn load(&self) -> ProjectsFile {
        let path = self.projects_path();
        let Ok(raw) = fs::read_to_string(&path) else {
            return ProjectsFile::default();
        };
        match serde_json::from_str::<ProjectsFile>(&raw) {
            Ok(file) => file,
            Err(error) => {
                // Keep the unreadable file aside instead of overwriting it on the next save.
                let backup = path.with_extension(format!("broken-{}.json", now_ms()));
                let _ = fs::copy(&path, backup);
                eprintln!("projects.json ilegível: {error}");
                ProjectsFile::default()
            }
        }
    }

    pub fn save(&self, file: &ProjectsFile) -> Result<(), String> {
        let content = serde_json::to_string(file).map_err(|e| e.to_string())?;
        atomic_write(&self.projects_path(), &content)
    }

    /// Replaces the native data with a fresh copy of the Tauri build's file.
    pub fn reimport(&self) -> Result<ProjectsFile, String> {
        let source = Self::tauri_projects_path().ok_or("Alethe (Tauri) data not found")?;
        let raw = fs::read_to_string(source).map_err(|e| e.to_string())?;
        let file: ProjectsFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        atomic_write(&self.projects_path(), &raw)?;
        Ok(file)
    }
}
