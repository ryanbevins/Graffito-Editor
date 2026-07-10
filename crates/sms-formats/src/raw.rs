use serde::{Deserialize, Serialize};

use crate::{detect_raw_format, PreserveBytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawFormat {
    Unknown,
    Yaz0,
    Rarc,
    J3d,
    Bmg,
    Bti,
    Col,
    StagePlacement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFile {
    format: RawFormat,
    bytes: Vec<u8>,
}

impl RawFile {
    pub fn new(bytes: Vec<u8>) -> Self {
        let format = detect_raw_format(&bytes);
        Self { format, bytes }
    }

    pub fn with_format(format: RawFormat, bytes: Vec<u8>) -> Self {
        Self { format, bytes }
    }

    pub fn format(&self) -> RawFormat {
        self.format
    }
}

impl PreserveBytes for RawFile {
    fn source_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_j3d_magic() {
        let file = RawFile::new(b"J3D2bmd3".to_vec());
        assert_eq!(file.format(), RawFormat::J3d);
        assert_eq!(file.to_bytes(), b"J3D2bmd3");
    }
}
