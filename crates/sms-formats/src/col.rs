use serde::{Deserialize, Serialize};

use crate::binary::{be_u32, require_len};
use crate::{FormatError, PreserveBytes, Result};

const FORMAT: &str = "COL";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColHeader {
    pub triangle_count_or_flags: u32,
    pub vertex_offset: u32,
    pub group_count: u32,
    pub group_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColFile {
    header: ColHeader,
    bytes: Vec<u8>,
}

impl ColFile {
    pub fn parse(bytes: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = bytes.as_ref();
        require_len(FORMAT, bytes, 0x10)?;
        let header = ColHeader {
            triangle_count_or_flags: be_u32(bytes, 0x00, FORMAT)?,
            vertex_offset: be_u32(bytes, 0x04, FORMAT)?,
            group_count: be_u32(bytes, 0x08, FORMAT)?,
            group_offset: be_u32(bytes, 0x0C, FORMAT)?,
        };

        for offset in [header.vertex_offset, header.group_offset] {
            if offset as usize >= bytes.len() {
                return Err(FormatError::InvalidOffset {
                    format: FORMAT,
                    offset: offset as usize,
                    len: bytes.len(),
                });
            }
        }

        Ok(Self {
            header,
            bytes: bytes.to_vec(),
        })
    }

    pub fn header(&self) -> &ColHeader {
        &self.header
    }
}

impl PreserveBytes for ColFile {
    fn source_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sms_collision_header_shape() {
        let mut bytes = vec![0; 0x30];
        bytes[4..8].copy_from_slice(&(0x10u32.to_be_bytes()));
        bytes[12..16].copy_from_slice(&(0x20u32.to_be_bytes()));

        let col = ColFile::parse(&bytes).unwrap();
        assert_eq!(col.header().vertex_offset, 0x10);
        assert_eq!(col.to_bytes(), bytes);
    }
}
