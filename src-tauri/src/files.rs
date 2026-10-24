use crate::domain::*;
use std::io::Read;
use zeroize::Zeroizing;

pub fn read_bounded(path: &str, limit: usize) -> AppResult<Zeroizing<Vec<u8>>> {
    let failure =
        || AppError::invalid("Choose a readable regular file within the 1 MiB file limit");
    if !std::fs::metadata(path).map_err(|_| failure())?.is_file() {
        return Err(failure());
    }
    let file = std::fs::File::open(path).map_err(|_| failure())?;
    let metadata = file.metadata().map_err(|_| failure())?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(failure());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() > limit {
        return Err(failure());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_bounded_regular_files_and_rejects_devices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture");
        std::fs::write(&path, b"certificate").unwrap();
        assert_eq!(
            &**super::read_bounded(path.to_str().unwrap(), 1024).unwrap(),
            b"certificate"
        );
        assert!(super::read_bounded(path.to_str().unwrap(), 3).is_err());
        #[cfg(unix)]
        assert!(super::read_bounded("/dev/zero", 1024).is_err());
    }
}
