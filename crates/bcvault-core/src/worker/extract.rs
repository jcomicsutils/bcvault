use crate::error::DownloadError;
use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::{Component, Path, PathBuf},
};

const MAX_ZIP_ENTRIES: usize = 2_000;
const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB

pub async fn extract_zip_archive(
    zip_path: PathBuf,
    delete_after_extract: bool,
) -> Result<PathBuf, DownloadError> {
    tokio::task::spawn_blocking(move || extract_zip_sync(&zip_path, delete_after_extract))
        .await
        .map_err(|e| DownloadError::Io(std::io::Error::other(e.to_string())))?
}

pub fn extract_zip_sync(
    zip_path: &Path,
    delete_after_extract: bool,
) -> Result<PathBuf, DownloadError> {
    let parent = zip_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let stem = zip_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("extracted");

    let target_dir = unique_dir_sync(&parent, stem);

    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file))?;

    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(DownloadError::ZipLimitExceeded(format!(
            "Archive has {} entries (max {})",
            archive.len(),
            MAX_ZIP_ENTRIES
        )));
    }

    // First pass: validate all entry paths and total uncompressed size before creating files
    let mut total_uncompressed: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let raw_name = entry.name().to_string();
        let Some(rel_path) = entry.enclosed_name() else {
            return Err(DownloadError::UnsafeZipEntry(raw_name));
        };
        if has_unsafe_component(&rel_path) {
            return Err(DownloadError::UnsafeZipEntry(raw_name));
        }
        total_uncompressed = total_uncompressed.saturating_add(entry.size());
        if total_uncompressed > MAX_TOTAL_UNCOMPRESSED_BYTES {
            return Err(DownloadError::ZipLimitExceeded(format!(
                "Archive uncompressed size exceeds {MAX_TOTAL_UNCOMPRESSED_BYTES} bytes"
            )));
        }
    }

    std::fs::create_dir_all(&target_dir)?;

    let extract_res: Result<(), DownloadError> = (|| {
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let Some(rel_path) = entry.enclosed_name() else {
                return Err(DownloadError::UnsafeZipEntry(entry.name().to_string()));
            };
            let out_path = target_dir.join(rel_path);

            if entry.is_dir() {
                std::fs::create_dir_all(&out_path)?;
            } else {
                if let Some(p) = out_path.parent() {
                    std::fs::create_dir_all(p)?;
                }
                let out_file = File::create(&out_path)?;
                let mut writer = BufWriter::with_capacity(64 * 1024, out_file);
                std::io::copy(&mut entry, &mut writer)?;
                std::io::Write::flush(&mut writer)?;
            }
        }
        Ok(())
    })();

    if let Err(e) = extract_res {
        let _ = std::fs::remove_dir_all(&target_dir);
        return Err(e);
    }

    drop(archive);

    if delete_after_extract {
        let _ = std::fs::remove_file(zip_path);
    }

    Ok(target_dir)
}

fn has_unsafe_component(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn unique_dir_sync(parent: &Path, stem: &str) -> PathBuf {
    let initial = parent.join(stem);
    if !initial.exists() {
        return initial;
    }
    for i in 1..10_000 {
        let candidate = parent.join(format!("{stem} ({i})"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}_{}", chrono::Utc::now().timestamp_millis()))
}
