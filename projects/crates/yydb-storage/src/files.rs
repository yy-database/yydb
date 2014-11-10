//! File lifecycle helpers shared by backup and compaction paths.

use std::{fs, path::Path};

use yydb_types::Result;

use crate::journal::{shm_path, wal_path};

/// Copy a main database image, creating the destination parent directory.
pub fn copy_main(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dest)?;
    Ok(())
}

/// Copy `-wal` / `-shm` sidecars when present.
pub fn copy_sidecars(src_db: &Path, dest_db: &Path) -> Result<()> {
    copy_if_present(&wal_path(src_db), &wal_path(dest_db))?;
    copy_if_present(&shm_path(src_db), &shm_path(dest_db))?;
    Ok(())
}

/// Recursively copy a blob tree when the source exists.
pub fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        }
        else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn copy_if_present(src: &Path, dest: &Path) -> Result<()> {
    if src.exists() {
        fs::copy(src, dest)?;
    }
    Ok(())
}
