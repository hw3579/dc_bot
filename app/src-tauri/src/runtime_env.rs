use std::path::{Path, PathBuf};
use std::sync::Once;

const DOTENV_SEARCH_PATHS: [&str; 3] = [".env", "../.env", "../../.env"];

static ENV_FILE_LOADED: Once = Once::new();

pub fn load_dotenv() {
    ENV_FILE_LOADED.call_once(|| {
        for path in dotenv_candidates() {
            if dotenvy::from_path(&path).is_ok() {
                break;
            }
        }
    });
}

pub fn env_string(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn dotenv_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(current_dir) = std::env::current_dir() {
        append_dotenv_candidates(&mut candidates, &current_dir);
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            append_dotenv_candidates(&mut candidates, exe_dir);
        }
    }

    candidates
}

fn append_dotenv_candidates(candidates: &mut Vec<PathBuf>, base_dir: &Path) {
    for relative_path in DOTENV_SEARCH_PATHS {
        let candidate = base_dir.join(relative_path);

        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
}