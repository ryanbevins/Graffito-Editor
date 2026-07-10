use serde::{Deserialize, Serialize};

use crate::binary::{be_u16, be_u32, checked_slice, require_len, require_magic};
use crate::{FormatError, PreserveBytes, Result};

const FORMAT: &str = "RARC";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RarcHeader {
    pub file_size: u32,
    pub header_size: u32,
    pub data_offset: u32,
    pub data_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RarcFileEntry {
    pub path: String,
    pub flags: u8,
    pub data_offset: u32,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RarcArchive {
    header: RarcHeader,
    bytes: Vec<u8>,
}

impl RarcArchive {
    pub fn parse(bytes: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = bytes.as_ref();
        require_magic(FORMAT, bytes, b"RARC")?;

        let header = RarcHeader {
            file_size: be_u32(bytes, 0x04, FORMAT)?,
            header_size: be_u32(bytes, 0x08, FORMAT)?,
            data_offset: be_u32(bytes, 0x0C, FORMAT)?,
            data_size: be_u32(bytes, 0x10, FORMAT)?,
        };

        Ok(Self {
            header,
            bytes: bytes.to_vec(),
        })
    }

    pub fn header(&self) -> &RarcHeader {
        &self.header
    }

    pub fn files(&self) -> Result<Vec<RarcFileEntry>> {
        let tables = RarcTables::parse(&self.bytes)?;
        let mut files = Vec::new();
        let mut visited_nodes = vec![false; tables.nodes.len()];
        walk_node(0, "", &tables, &mut visited_nodes, &mut files)?;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    pub fn file_bytes(&self, archive_path: &str) -> Result<Vec<u8>> {
        let normalized = archive_path.trim_start_matches('/').replace('\\', "/");
        let entry = self
            .files()?
            .into_iter()
            .find(|entry| entry.path == normalized)
            .ok_or_else(|| FormatError::Unsupported {
                format: FORMAT,
                message: format!("archive entry not found: {archive_path}"),
            })?;

        let data_start = (self.header.header_size as usize)
            .checked_add(self.header.data_offset as usize)
            .and_then(|offset| offset.checked_add(entry.data_offset as usize))
            .ok_or_else(|| invalid_offset(entry.data_offset as usize, self.bytes.len()))?;
        let bytes = checked_slice(FORMAT, &self.bytes, data_start, entry.size as usize)?;
        Ok(bytes.to_vec())
    }
}

impl PreserveBytes for RarcArchive {
    fn source_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone)]
struct RarcTables {
    nodes: Vec<RarcNode>,
    entries: Vec<RarcEntry>,
}

#[derive(Debug, Clone)]
struct RarcNode {
    entry_count: u16,
    first_entry_index: u32,
}

#[derive(Debug, Clone)]
struct RarcEntry {
    name: String,
    flags: u8,
    data_offset: u32,
    size: u32,
}

impl RarcEntry {
    fn is_directory(&self) -> bool {
        (self.flags & 0x02) != 0
    }

    fn is_dot_entry(&self) -> bool {
        self.name == "." || self.name == ".."
    }
}

impl RarcTables {
    fn parse(bytes: &[u8]) -> Result<Self> {
        require_magic(FORMAT, bytes, b"RARC")?;
        require_len(FORMAT, bytes, 0x40)?;

        let info_offset = be_u32(bytes, 0x08, FORMAT)? as usize;
        require_len(FORMAT, bytes, info_offset + 0x20)?;

        let node_count = be_u32(bytes, info_offset, FORMAT)? as usize;
        let node_offset = info_relative_offset(bytes, info_offset, 0x04)?;
        let file_entry_count = be_u32(bytes, info_offset + 0x08, FORMAT)? as usize;
        let file_entry_offset = info_relative_offset(bytes, info_offset, 0x0C)?;
        let string_table_length = be_u32(bytes, info_offset + 0x10, FORMAT)? as usize;
        let string_table_offset = info_relative_offset(bytes, info_offset, 0x14)?;
        let string_table =
            checked_slice(FORMAT, bytes, string_table_offset, string_table_length)?.to_vec();

        let mut nodes = Vec::with_capacity(node_count);
        for index in 0..node_count {
            let offset = node_offset
                .checked_add(index * 0x10)
                .ok_or_else(|| invalid_offset(node_offset, bytes.len()))?;
            require_len(FORMAT, bytes, offset + 0x10)?;
            let _node_type = be_u32(bytes, offset, FORMAT)?;
            let _name_offset = be_u32(bytes, offset + 0x04, FORMAT)?;
            let _name_hash = be_u16(bytes, offset + 0x08, FORMAT)?;
            nodes.push(RarcNode {
                entry_count: be_u16(bytes, offset + 0x0A, FORMAT)?,
                first_entry_index: be_u32(bytes, offset + 0x0C, FORMAT)?,
            });
        }

        let mut entries = Vec::with_capacity(file_entry_count);
        for index in 0..file_entry_count {
            let offset = file_entry_offset
                .checked_add(index * 0x14)
                .ok_or_else(|| invalid_offset(file_entry_offset, bytes.len()))?;
            require_len(FORMAT, bytes, offset + 0x14)?;

            let _file_id = be_u16(bytes, offset, FORMAT)?;
            let _name_hash = be_u16(bytes, offset + 0x02, FORMAT)?;
            let flags_and_name_offset = be_u32(bytes, offset + 0x04, FORMAT)?;
            let flags = (flags_and_name_offset >> 24) as u8;
            let name_offset = (flags_and_name_offset & 0x00FF_FFFF) as usize;
            let name = read_string(&string_table, name_offset)?;
            entries.push(RarcEntry {
                name,
                flags,
                data_offset: be_u32(bytes, offset + 0x08, FORMAT)?,
                size: be_u32(bytes, offset + 0x0C, FORMAT)?,
            });
        }

        Ok(Self { nodes, entries })
    }
}

fn walk_node(
    node_index: usize,
    prefix: &str,
    tables: &RarcTables,
    visited_nodes: &mut [bool],
    files: &mut Vec<RarcFileEntry>,
) -> Result<()> {
    if node_index >= tables.nodes.len() {
        return Err(invalid_offset(node_index, tables.nodes.len()));
    }
    if visited_nodes[node_index] {
        return Ok(());
    }
    visited_nodes[node_index] = true;

    let node = &tables.nodes[node_index];
    let start = node.first_entry_index as usize;
    let end = start
        .checked_add(node.entry_count as usize)
        .ok_or_else(|| invalid_offset(start, tables.entries.len()))?;
    if end > tables.entries.len() {
        return Err(invalid_offset(end, tables.entries.len()));
    }

    for entry in &tables.entries[start..end] {
        if entry.is_dot_entry() {
            continue;
        }

        let path = join_archive_path(prefix, &entry.name);
        if entry.is_directory() {
            walk_node(
                entry.data_offset as usize,
                &path,
                tables,
                visited_nodes,
                files,
            )?;
        } else {
            files.push(RarcFileEntry {
                path,
                flags: entry.flags,
                data_offset: entry.data_offset,
                size: entry.size,
            });
        }
    }

    Ok(())
}

fn info_relative_offset(bytes: &[u8], info_offset: usize, field_offset: usize) -> Result<usize> {
    let relative = be_u32(bytes, info_offset + field_offset, FORMAT)? as usize;
    info_offset
        .checked_add(relative)
        .ok_or_else(|| invalid_offset(relative, bytes.len()))
}

fn read_string(string_table: &[u8], offset: usize) -> Result<String> {
    if offset >= string_table.len() {
        return Err(invalid_offset(offset, string_table.len()));
    }

    let tail = &string_table[offset..];
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| invalid_offset(offset, string_table.len()))?;
    Ok(String::from_utf8_lossy(&tail[..end]).to_string())
}

fn join_archive_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn invalid_offset(offset: usize, len: usize) -> FormatError {
    FormatError::InvalidOffset {
        format: FORMAT,
        offset,
        len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_minimal_rarc_bytes() {
        let mut bytes = vec![0; 0x20];
        bytes[0..4].copy_from_slice(b"RARC");
        bytes[4..8].copy_from_slice(&(0x20u32.to_be_bytes()));
        bytes[8..12].copy_from_slice(&(0x20u32.to_be_bytes()));

        let archive = RarcArchive::parse(&bytes).unwrap();
        assert_eq!(archive.header().file_size, 0x20);
        assert_eq!(archive.to_bytes(), bytes);
    }

    #[test]
    fn lists_file_entries_from_rarc_tables() {
        let string_table = b"root\0.\0..\0map.bmd\0";
        let root_name = 0u32;
        let dot_name = 5u32;
        let dotdot_name = 7u32;
        let file_name = 10u32;

        let info_offset = 0x20usize;
        let node_offset = 0x20usize;
        let file_entry_offset = 0x30usize;
        let string_table_offset = 0x6Cusize;
        let file_data_offset = string_table_offset + string_table.len();
        let file_size = info_offset + file_data_offset + 4;

        let mut bytes = vec![0; file_size];
        bytes[0..4].copy_from_slice(b"RARC");
        bytes[4..8].copy_from_slice(&(file_size as u32).to_be_bytes());
        bytes[8..12].copy_from_slice(&(info_offset as u32).to_be_bytes());
        bytes[12..16].copy_from_slice(&(file_data_offset as u32).to_be_bytes());
        bytes[16..20].copy_from_slice(&(4u32.to_be_bytes()));

        bytes[info_offset..info_offset + 4].copy_from_slice(&(1u32.to_be_bytes()));
        bytes[info_offset + 4..info_offset + 8]
            .copy_from_slice(&(node_offset as u32).to_be_bytes());
        bytes[info_offset + 8..info_offset + 12].copy_from_slice(&(3u32.to_be_bytes()));
        bytes[info_offset + 12..info_offset + 16]
            .copy_from_slice(&(file_entry_offset as u32).to_be_bytes());
        bytes[info_offset + 16..info_offset + 20]
            .copy_from_slice(&(string_table.len() as u32).to_be_bytes());
        bytes[info_offset + 20..info_offset + 24]
            .copy_from_slice(&(string_table_offset as u32).to_be_bytes());

        let node = info_offset + node_offset;
        bytes[node..node + 4].copy_from_slice(b"ROOT");
        bytes[node + 4..node + 8].copy_from_slice(&root_name.to_be_bytes());
        bytes[node + 10..node + 12].copy_from_slice(&(3u16.to_be_bytes()));

        write_entry(
            &mut bytes,
            info_offset + file_entry_offset,
            0x02,
            dot_name,
            0,
            0,
        );
        write_entry(
            &mut bytes,
            info_offset + file_entry_offset + 0x14,
            0x02,
            dotdot_name,
            0,
            0,
        );
        write_entry(
            &mut bytes,
            info_offset + file_entry_offset + 0x28,
            0x11,
            file_name,
            0,
            4,
        );
        bytes[info_offset + string_table_offset
            ..info_offset + string_table_offset + string_table.len()]
            .copy_from_slice(string_table);

        let archive = RarcArchive::parse(&bytes).unwrap();
        let files = archive.files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "map.bmd");
        assert_eq!(files[0].size, 4);
    }

    fn write_entry(
        bytes: &mut [u8],
        offset: usize,
        flags: u8,
        name_offset: u32,
        data_offset: u32,
        size: u32,
    ) {
        let flags_and_name = ((flags as u32) << 24) | name_offset;
        bytes[offset + 4..offset + 8].copy_from_slice(&flags_and_name.to_be_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&data_offset.to_be_bytes());
        bytes[offset + 12..offset + 16].copy_from_slice(&size.to_be_bytes());
    }
}
