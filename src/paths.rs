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
        Ok(())
    }
}
