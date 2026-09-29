use std::path::Path;

use crate::config::Config;
use crate::rules::CleanupRule;
use crate::scanner::entry::{Category, SafetyLevel, ScannedEntry};
use crate::scanner::walker;

const MIN_SIZE: u64 = 1_048_576; // 1 MiB

const ELECTRON_APPS: &[(&str, &str)] = &[
    ("Slack", "Slack"),
    ("Discord", "discord"),
    ("VS Code", "Code"),
    ("Cursor", "Cursor"),
    ("Teams", "Microsoft/Teams"),
];

/// Chromium profile directories, and what removing each one costs.
///
/// Only the caches are Safe: the monitor's auto-clean deletes Safe entries
/// unattended. `Local Storage` and `IndexedDB` are where these apps keep the
/// signed-in session and offline data, so removing them signs you out.
const ELECTRON_SUBDIRS: &[(&str, SafetyLevel, &str)] = &[
    ("GPUCache", SafetyLevel::Safe, "GPU shader cache"),
    ("Code Cache", SafetyLevel::Safe, "compiled script cache"),
    (
        "Service Worker/CacheStorage",
        SafetyLevel::Safe,
        "service worker cache",
    ),
    (
        "Service Worker/ScriptCache",
        SafetyLevel::Safe,
        "service worker script cache",
    ),
    ("blob_storage", SafetyLevel::Safe, "blob storage"),
    (
        "Local Storage",
        SafetyLevel::Caution,
        "Local Storage (sign-in and settings)",
    ),
    (
        "IndexedDB",
        SafetyLevel::Caution,
        "IndexedDB (sign-in and offline data)",
    ),
    (
        "Session Storage",
        SafetyLevel::Caution,
        "Session Storage (open window state)",
    ),
];

pub struct ElectronAppDataRule;

impl CleanupRule for ElectronAppDataRule {
    fn name(&self) -> &'static str {
        "Electron app data"
    }

    fn category(&self) -> Category {
        Category::AppCache
    }

    fn scan(&self, _config: &Config) -> Vec<ScannedEntry> {
        let home = dirs::home_dir().unwrap_or_default();
        let mut entries = Vec::new();

        // macOS: ~/Library/Application Support/<app>
        let macos_dir = home.join("Library/Application Support");
        if macos_dir.exists() {
            for &(app_name, app_dir) in ELECTRON_APPS {
                let base = macos_dir.join(app_dir);
                if !base.exists() {
                    continue;
                }
                for &(subdir, safety, what) in ELECTRON_SUBDIRS {
                    scan_subdir(&mut entries, &base.join(subdir), safety, app_name, what);
                }
            }
        }

        // Linux: ~/.config/<app>
        let linux_dir = home.join(".config");
        if linux_dir.exists() {
            for &(app_name, app_dir) in ELECTRON_APPS {
                let base = linux_dir.join(app_dir);
                if !base.exists() {
                    continue;
                }
                for &(subdir, safety, what) in ELECTRON_SUBDIRS {
                    scan_subdir(&mut entries, &base.join(subdir), safety, app_name, what);
                }
            }
        }

        entries
    }
}

fn scan_subdir(
    entries: &mut Vec<ScannedEntry>,
    path: &Path,
    safety: SafetyLevel,
    app: &str,
    what: &str,
) {
    if !path.exists() {
        return;
    }

    let size = walker::dir_size(path);
    if size < MIN_SIZE {
        return;
    }

    entries.push(ScannedEntry {
        path: path.to_path_buf(),
        size,
        category: Category::AppCache,
        safety,
        description: format!("{app} {what}"),
        item_count: None,
    });
}

pub fn rules() -> Vec<Box<dyn CleanupRule>> {
    vec![Box::new(ElectronAppDataRule)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_is_never_offered_as_a_safe_cache() {
        // Safe entries are deleted unattended by the monitor's auto-clean.
        for &(subdir, safety, _) in ELECTRON_SUBDIRS {
            if matches!(subdir, "Local Storage" | "IndexedDB" | "Session Storage") {
                assert_ne!(safety, SafetyLevel::Safe, "{subdir} holds app state");
            }
        }
    }
}
