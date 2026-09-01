//! Short-lived UTF-8 transport spool used by the PowerShell filter boundary.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SPOOL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct OutputSpool {
    path: PathBuf,
}

impl OutputSpool {
    pub fn create() -> io::Result<Self> {
        let directory = std::env::temp_dir();
        for _ in 0..32 {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let sequence = SPOOL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!("rtk-powershell-{timestamp:x}-{sequence:x}.utf8"));
            match std::fs::OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "unable to allocate a unique PowerShell spool path",
        ))
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        file.write_all(bytes)
    }

    pub(crate) fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let mut file = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        file.write_all(bytes)
    }

    pub fn read_utf8(&self) -> io::Result<String> {
        let mut file = std::fs::File::open(&self.path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for OutputSpool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path); // nosemgrep: filesystem-deletion -- removes only RTK's temporary spool.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spool_round_trips_utf8_and_cleans_up() {
        let spool = OutputSpool::create().expect("spool");
        let path = spool.path().to_path_buf();
        spool.write("Прод\r\n資料".as_bytes()).expect("write");
        assert_eq!(spool.read_utf8().expect("read"), "Прод\r\n資料");
        drop(spool);
        assert!(!path.exists());
    }
}
