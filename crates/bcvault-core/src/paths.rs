use directories::ProjectDirs;
use std::{
    fs::{File, OpenOptions},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub db_path: PathBuf,
    pub fx_db_path: PathBuf,
    pub cover_cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub themes_dir: PathBuf,
    pub default_download_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Option<Self> {
        let mut args = std::env::args().skip(1);
        let mut cli_override: Option<PathBuf> = None;
        while let Some(arg) = args.next() {
            if arg == "--data-dir" {
                if let Some(val) = args.next() {
                    cli_override = Some(PathBuf::from(val));
                }
            } else if let Some(val) = arg.strip_prefix("--data-dir=") {
                cli_override = Some(PathBuf::from(val));
            }
        }
        let env_override = std::env::var_os("BCVAULT_DATA_DIR").map(PathBuf::from);
        if let Some(custom_dir) = cli_override.or(env_override) {
            return Some(Self::from_base_dirs(
                custom_dir.clone(),
                custom_dir.join("cache"),
            ));
        }

        let proj = ProjectDirs::from("org", "bcvault", "bcvault")?;
        Some(Self::from_base_dirs(
            proj.data_dir().to_path_buf(),
            proj.cache_dir().to_path_buf(),
        ))
    }

    pub fn from_base_dirs(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        let db_path = data_dir.join("bcvault.db");
        let fx_db_path = data_dir.join("fx.sqlite");
        let cover_cache_dir = cache_dir.join("covers");
        let log_dir = data_dir.join("logs");
        let themes_dir = data_dir.join("themes");
        let default_download_dir = directories::UserDirs::new()
            .and_then(|u| {
                u.audio_dir()
                    .map(|p| p.to_path_buf())
                    .or_else(|| u.download_dir().map(|p| p.to_path_buf()))
            })
            .unwrap_or_else(|| data_dir.join("downloads"))
            .join("bcvault");

        Self {
            data_dir,
            cache_dir,
            db_path,
            fx_db_path,
            cover_cache_dir,
            log_dir,
            themes_dir,
            default_download_dir,
        }
    }

    pub fn acquire_instance_lock(&self) -> io::Result<InstanceLock> {
        acquire_instance_lock_at(&self.data_dir)
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(&self.cover_cache_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        std::fs::create_dir_all(&self.themes_dir)?;
        self.rotate_logs(5 * 1024 * 1024, 5)?;
        Ok(())
    }

    pub fn rotate_logs(&self, max_bytes: u64, keep_files: usize) -> std::io::Result<()> {
        let current = self.log_dir.join("bcvault.log");
        if let Ok(meta) = std::fs::metadata(&current)
            && meta.len() >= max_bytes
        {
            for idx in (1..keep_files).rev() {
                let src = self.log_dir.join(format!("bcvault.log.{idx}"));
                let dst = self.log_dir.join(format!("bcvault.log.{}", idx + 1));
                if src.exists() {
                    let _ = std::fs::rename(src, dst);
                }
            }
            let first_rot = self.log_dir.join("bcvault.log.1");
            let _ = std::fs::rename(&current, first_rot);
        }
        Ok(())
    }
}

/// Converts a path to an extended-length path on Windows when needed so long artist/release
/// paths do not fail MAX_PATH (260 chars). On non-Windows platforms returns the path unchanged.
pub fn to_os_safe_path(path: &std::path::Path) -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(canon) = std::fs::canonicalize(path) {
            return canon;
        }
        let s = path.to_string_lossy();
        if s.len() >= 240 && !s.starts_with(r"\\?\") {
            return PathBuf::from(format!(r"\\?\{s}"));
        }
    }
    path.to_path_buf()
}

pub fn acquire_instance_lock_at(data_dir: &Path) -> io::Result<InstanceLock> {
    std::fs::create_dir_all(data_dir)?;
    let lock_path = data_dir.join("bcvault.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    match file.try_lock() {
        Ok(()) => Ok(InstanceLock {
            _file: file,
            path: lock_path,
        }),
        Err(_) => Err(io::Error::new(
            ErrorKind::WouldBlock,
            format!(
                "Another instance of bcvault or bcvault-fx is already running (lock held on {})",
                lock_path.display()
            ),
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn instance_lock_prevents_second_acquire_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let lock1 = acquire_instance_lock_at(dir.path()).unwrap();
        let err = acquire_instance_lock_at(dir.path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
        assert!(err.to_string().contains("already running"));
        drop(lock1);
        let _lock2 = acquire_instance_lock_at(dir.path()).unwrap();
    }
}
