use crate::{FormatError, PreserveBytes, Result};

const FORMAT: &str = "SMS parameter file";
const MAX_ENTRIES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrmEntry {
    pub key_code: u16,
    pub name: String,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrmFile {
    bytes: Vec<u8>,
    pub entries: Vec<PrmEntry>,
}

impl PrmFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(FormatError::TooSmall {
                format: FORMAT,
                expected: 4,
                actual: bytes.len(),
            });
        }
        let entry_count = read_u32(bytes, 0)? as usize;
        if entry_count > MAX_ENTRIES {
            return Err(FormatError::ResourceLimit {
                format: FORMAT,
                resource: "entries",
                requested: entry_count,
                limit: MAX_ENTRIES,
            });
        }
        let mut offset = 4usize;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let key_code = read_u16(bytes, offset)?;
            let name_len = read_u16(bytes, offset + 2)? as usize;
            offset = offset.checked_add(4).ok_or(FormatError::InvalidOffset {
                format: FORMAT,
                offset,
                len: bytes.len(),
            })?;
            let name_bytes = checked_slice(bytes, offset, name_len)?;
            let name = std::str::from_utf8(name_bytes)
                .map_err(|error| FormatError::Unsupported {
                    format: FORMAT,
                    message: format!("parameter name at {offset:#x} is not UTF-8: {error}"),
                })?
                .to_string();
            offset = offset
                .checked_add(name_len)
                .ok_or(FormatError::InvalidOffset {
                    format: FORMAT,
                    offset,
                    len: bytes.len(),
                })?;
            let value_len = read_u32(bytes, offset)? as usize;
            offset = offset.checked_add(4).ok_or(FormatError::InvalidOffset {
                format: FORMAT,
                offset,
                len: bytes.len(),
            })?;
            let value = checked_slice(bytes, offset, value_len)?.to_vec();
            offset = offset
                .checked_add(value_len)
                .ok_or(FormatError::InvalidOffset {
                    format: FORMAT,
                    offset,
                    len: bytes.len(),
                })?;
            entries.push(PrmEntry {
                key_code,
                name,
                value,
            });
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            entries,
        })
    }

    pub fn f32(&self, name: &str) -> Option<f32> {
        let entry = self.entries.iter().find(|entry| entry.name == name)?;
        let bytes: [u8; 4] = entry.value.as_slice().try_into().ok()?;
        Some(f32::from_bits(u32::from_be_bytes(bytes)))
    }
}

impl PreserveBytes for PrmFile {
    fn source_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let value: [u8; 2] = checked_slice(bytes, offset, 2)?
        .try_into()
        .expect("checked two-byte slice");
    Ok(u16::from_be_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value: [u8; 4] = checked_slice(bytes, offset, 4)?
        .try_into()
        .expect("checked four-byte slice");
    Ok(u32::from_be_bytes(value))
}

fn checked_slice(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    let end = offset.checked_add(len).ok_or(FormatError::InvalidOffset {
        format: FORMAT,
        offset,
        len: bytes.len(),
    })?;
    bytes.get(offset..end).ok_or(FormatError::InvalidOffset {
        format: FORMAT,
        offset,
        len: bytes.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_named_big_endian_float_parameters_losslessly() {
        let mut bytes = 2u32.to_be_bytes().to_vec();
        for (key, name, value) in [
            (0x878e_u16, "mSLBodyScaleLow", 1.0_f32),
            (0x9640_u16, "mSLBodyScaleHigh", 1.25_f32),
        ] {
            bytes.extend_from_slice(&key.to_be_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(&4u32.to_be_bytes());
            bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        }

        let file = PrmFile::parse(&bytes).expect("parse parameter fixture");
        assert_eq!(file.f32("mSLBodyScaleLow"), Some(1.0));
        assert_eq!(file.f32("mSLBodyScaleHigh"), Some(1.25));
        assert_eq!(file.to_bytes(), bytes);
    }

    #[test]
    fn rejects_truncated_parameter_values() {
        let bytes = [0, 0, 0, 1, 0, 1, 0, 1, b'x', 0, 0, 0, 4, 0, 0];
        assert!(matches!(
            PrmFile::parse(&bytes),
            Err(FormatError::InvalidOffset { .. })
        ));
    }
}
