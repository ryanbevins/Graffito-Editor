use serde::{Deserialize, Serialize};

use crate::binary::{be_u32, checked_slice, require_magic};
use crate::{FormatError, PreserveBytes, Result};

const FORMAT: &str = "Yaz0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Yaz0File {
    decompressed_size: u32,
    bytes: Vec<u8>,
}

impl Yaz0File {
    pub fn parse(bytes: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = bytes.as_ref();
        require_magic(FORMAT, bytes, b"Yaz0")?;
        let decompressed_size = be_u32(bytes, 0x04, FORMAT)?;
        Ok(Self {
            decompressed_size,
            bytes: bytes.to_vec(),
        })
    }

    pub fn decompressed_size(&self) -> u32 {
        self.decompressed_size
    }

    pub fn decode(&self) -> Result<Vec<u8>> {
        decode_yaz0(&self.bytes)
    }
}

impl PreserveBytes for Yaz0File {
    fn source_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub fn decode_yaz0(bytes: &[u8]) -> Result<Vec<u8>> {
    require_magic(FORMAT, bytes, b"Yaz0")?;
    let decoded_size = be_u32(bytes, 0x04, FORMAT)? as usize;
    checked_slice(FORMAT, bytes, 0x10, 0)?;

    let mut src = 0x10;
    let mut dst = Vec::with_capacity(decoded_size);

    while dst.len() < decoded_size {
        if src >= bytes.len() {
            return Err(FormatError::InvalidOffset {
                format: FORMAT,
                offset: src,
                len: bytes.len(),
            });
        }

        let code = bytes[src];
        src += 1;

        for bit in 0..8 {
            if dst.len() >= decoded_size {
                break;
            }

            if (code & (0x80 >> bit)) != 0 {
                if src >= bytes.len() {
                    return Err(FormatError::InvalidOffset {
                        format: FORMAT,
                        offset: src,
                        len: bytes.len(),
                    });
                }
                dst.push(bytes[src]);
                src += 1;
            } else {
                if src + 1 >= bytes.len() {
                    return Err(FormatError::InvalidOffset {
                        format: FORMAT,
                        offset: src,
                        len: bytes.len(),
                    });
                }

                let byte1 = bytes[src];
                let byte2 = bytes[src + 1];
                src += 2;

                let dist = ((((byte1 & 0x0F) as usize) << 8) | byte2 as usize) + 1;
                let mut count = (byte1 >> 4) as usize;
                if count == 0 {
                    if src >= bytes.len() {
                        return Err(FormatError::InvalidOffset {
                            format: FORMAT,
                            offset: src,
                            len: bytes.len(),
                        });
                    }
                    count = bytes[src] as usize + 0x12;
                    src += 1;
                } else {
                    count += 2;
                }

                if dist > dst.len() {
                    return Err(FormatError::Unsupported {
                        format: FORMAT,
                        message: format!("back-reference distance {dist} before start"),
                    });
                }

                for _ in 0..count {
                    let value = dst[dst.len() - dist];
                    dst.push(value);
                    if dst.len() >= decoded_size {
                        break;
                    }
                }
            }
        }
    }

    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_literal_only_stream() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"Yaz0");
        bytes.extend_from_slice(&(3u32.to_be_bytes()));
        bytes.extend_from_slice(&[0; 8]);
        bytes.push(0xE0);
        bytes.extend_from_slice(b"SMS");

        let file = Yaz0File::parse(&bytes).unwrap();
        assert_eq!(file.decompressed_size(), 3);
        assert_eq!(file.decode().unwrap(), b"SMS");
        assert_eq!(file.to_bytes(), bytes);
    }
}
