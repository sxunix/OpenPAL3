use std::io::{Cursor, Read, Seek};

use mini_fs::UserFile;

type Loader = Box<dyn Fn() -> std::io::Result<Vec<u8>> + Send>;

pub struct MemoryFile {
    cursor: Cursor<Vec<u8>>,
    /// Set for a [`MemoryFile::deferred`] file until its first read.
    pending: Option<Loader>,
}

impl MemoryFile {
    pub fn new(cursor: Cursor<Vec<u8>>) -> MemoryFile {
        MemoryFile {
            cursor,
            pending: None,
        }
    }

    /// A file whose bytes `load` produces on the first read or seek.
    /// Opening a packed file only to see whether it exists (texture lookups
    /// probe `.dds` before `.tga`, per material) then costs no I/O and no
    /// decompression.
    pub fn deferred(load: impl Fn() -> std::io::Result<Vec<u8>> + Send + 'static) -> MemoryFile {
        MemoryFile {
            cursor: Cursor::new(vec![]),
            pending: Some(Box::new(load)),
        }
    }

    pub fn content(&self) -> Vec<u8> {
        match &self.pending {
            Some(load) => load().unwrap_or_default(),
            None => self.cursor.clone().into_inner(),
        }
    }

    fn ensure_loaded(&mut self) -> std::io::Result<()> {
        if let Some(load) = &self.pending {
            self.cursor = Cursor::new(load()?);
            self.pending = None;
        }
        Ok(())
    }
}

impl UserFile for MemoryFile {}

impl Read for MemoryFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.ensure_loaded()?;
        self.cursor.read(buf)
    }
}

impl Seek for MemoryFile {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.ensure_loaded()?;
        self.cursor.seek(pos)
    }
}
