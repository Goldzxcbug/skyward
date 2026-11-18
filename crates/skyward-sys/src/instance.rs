//! Hold an advisory lock for the lifetime of the recovery service.
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;

pub fn acquire(path: &Path) -> Result<File> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(path).with_context(|| format!("open {}", path.display()))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error()).context("another recovery instance is already running");
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_instance_is_rejected_until_the_first_releases_its_lock() {
        let path = std::env::temp_dir().join(format!("skyward-instance-{}.lock", std::process::id()));
        let first = acquire(&path).unwrap();
        assert!(acquire(&path).is_err());
        drop(first);
        let next = acquire(&path).unwrap();
        drop(next);
        std::fs::remove_file(path).unwrap();
    }
}
