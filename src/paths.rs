use directories::ProjectDirs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub db_path: PathBuf,
    pub cover_cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub default_download_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Option<Self> {
        let proj = ProjectDirs::from("org", "bcvault", "bcvault")?;
        let data_dir = proj.data_dir().to_path_buf();
        let cache_dir = proj.cache_dir().to_path_buf();
        let db_path = data_dir.join("bcvault.db");
        let cover_cache_dir = cache_dir.join("covers");
        let log_dir = data_dir.join("logs");
        let default_download_dir = directories::UserDirs::new()
            .and_then(|u| {
                u.audio_dir()
                    .map(|p| p.to_path_buf())
                    .or_else(|| u.download_dir().map(|p| p.to_path_buf()))
            })
            .unwrap_or_else(|| data_dir.join("downloads"))
            .join("bcvault");

        Some(Self {
            data_dir,
            cache_dir,
            db_path,
            cover_cache_dir,
            log_dir,
            default_download_dir,
        })
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(&self.cover_cache_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
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
