use std::collections::BTreeMap;

use encoding_rs::SHIFT_JIS;
use serde::{Deserialize, Serialize};

use crate::binary::{be_u16, be_u32, checked_slice, require_len, require_magic};
use crate::{FormatError, Result};

const FORMAT: &str = "BMG message archive";
const HEADER_SIZE: usize = 0x20;
const INFO_HEADER_SIZE: usize = 0x10;
const DATA_HEADER_SIZE: usize = 8;
const FILE_ALIGNMENT: usize = 0x20;
const MAX_MESSAGES: usize = 65_535;
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// The INF1 entry size consumed by Sunshine's `TMessageLoader::EntryInfo`.
pub const SMS_BMG_ENTRY_SIZE: u16 = 12;
/// `TMessageLoader` has a fixed array of 255 message entries.
pub const SMS_BMG_RUNTIME_MESSAGE_LIMIT: usize = 255;
/// `TTalk2D2::scTalkSoundList` has 135 entries, indexed `0..=134`.
pub const SMS_TALK_SOUND_LIMIT: usize = 135;
/// `TTalk2D2::setTagParam` stores each choice label in a 0x11-byte C string.
pub const SMS_BMG_CHOICE_TEXT_MAX_SHIFT_JIS_BYTES: usize = 16;

/// A source-free `MESGbmg1` message archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BmgFile {
    pub header_reserved: [u8; 16],
    pub info_section_size: u32,
    pub data_section_size: u32,
    pub entry_size: u16,
    pub group_id: u16,
    pub default_color: u8,
    pub info_reserved: u8,
    pub entries: Vec<BmgEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BmgEntry {
    /// Offset from the first byte after the DAT1 section header.
    pub message_offset: u32,
    /// Per-message INF1 attributes. Its length is `entry_size - 4`.
    pub attributes: Vec<u8>,
    pub message: BmgMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct BmgMessage {
    pub tokens: Vec<BmgMessageToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum BmgMessageToken {
    /// Shift-JIS text, represented as Unicode in the authoring model.
    Text(String),
    /// A BMG `0x1A` escape. The length byte is regenerated from the payload.
    Control(Vec<u8>),
}

/// A decomp-confirmed `TTalk2D2::setTagParam` control.
///
/// Unknown controls remain typed as an opaque payload so a source-free parse
/// and rebuild never loses future or context-specific control data.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SmsBmgControl {
    CharacterDelay(u8),
    AutomaticContinuation,
    Choice { slot: u8, text: String },
    DynamicValue(SmsBmgDynamicValue),
    FruitBasketRemaining(u8),
    Color(u8),
    Unknown(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmsBmgDynamicValue {
    TimerFlag20003,
    TimerFlag20002,
    RoundedFlag20004,
    BlueCoinTradeRemainder,
    TimerFlag20014,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BmgStableEdit {
    pub entry_index: usize,
    pub previous_offset: Option<u32>,
    pub message_offset: u32,
    pub layout: BmgStableEditLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmgStableEditLayout {
    PreservedOffset,
    AppendedPayload,
}

impl BmgMessageToken {
    pub fn sms_control(&self) -> Result<Option<SmsBmgControl>> {
        match self {
            Self::Text(_) => Ok(None),
            Self::Control(payload) => SmsBmgControl::decode(payload).map(Some),
        }
    }

    pub fn from_sms_control(control: SmsBmgControl) -> Result<Self> {
        Ok(Self::Control(control.encode_payload()?))
    }
}

impl SmsBmgControl {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        if payload.len() < 3 {
            return Err(unsupported(format!(
                "SMS control payload has {} bytes; expected at least type + id",
                payload.len()
            )));
        }
        let control_type = payload[0];
        let control_id = u16::from_be_bytes([payload[1], payload[2]]);
        let arguments = &payload[3..];
        let exact_arguments = |expected: usize| -> Result<()> {
            if arguments.len() != expected {
                return Err(unsupported(format!(
                    "SMS control type {control_type:#04x} id {control_id:#06x} has {} argument bytes; expected {expected}",
                    arguments.len()
                )));
            }
            Ok(())
        };
        match (control_type, control_id) {
            (0, 0) => {
                exact_arguments(1)?;
                Ok(Self::CharacterDelay(arguments[0]))
            }
            (0, 1) => {
                exact_arguments(0)?;
                Ok(Self::AutomaticContinuation)
            }
            (1, slot @ 0..=1) => {
                let text = SHIFT_JIS
                    .decode_without_bom_handling_and_without_replacement(arguments)
                    .ok_or_else(|| {
                        unsupported("choice control contains invalid Shift-JIS".to_string())
                    })?;
                Ok(Self::Choice {
                    slot: slot as u8,
                    text: text.into_owned(),
                })
            }
            (2, id @ (0 | 1 | 2 | 3 | 6)) => {
                exact_arguments(0)?;
                let value = match id {
                    0 => SmsBmgDynamicValue::TimerFlag20003,
                    1 => SmsBmgDynamicValue::TimerFlag20002,
                    2 => SmsBmgDynamicValue::RoundedFlag20004,
                    3 => SmsBmgDynamicValue::BlueCoinTradeRemainder,
                    6 => SmsBmgDynamicValue::TimerFlag20014,
                    _ => unreachable!(),
                };
                Ok(Self::DynamicValue(value))
            }
            (2, 4) => {
                exact_arguments(1)?;
                if arguments[0] > 3 {
                    return Err(unsupported(format!(
                        "SMS fruit-basket control index {} is outside 0..=3",
                        arguments[0]
                    )));
                }
                Ok(Self::FruitBasketRemaining(arguments[0]))
            }
            (0xFF, 0) => {
                exact_arguments(1)?;
                if arguments[0] >= 6 {
                    return Err(unsupported(format!(
                        "SMS text color {} is outside TTalk2D2's six-color table",
                        arguments[0]
                    )));
                }
                Ok(Self::Color(arguments[0]))
            }
            _ => Ok(Self::Unknown(payload.to_vec())),
        }
    }

    pub fn encode_payload(&self) -> Result<Vec<u8>> {
        let payload = match self {
            Self::CharacterDelay(delay) => vec![0, 0, 0, *delay],
            Self::AutomaticContinuation => vec![0, 0, 1],
            Self::Choice { slot, text } => {
                if *slot > 1 {
                    return Err(unsupported(format!(
                        "SMS dialogue choice slot {slot} is not 0 or 1"
                    )));
                }
                let (encoded, _, had_errors) = SHIFT_JIS.encode(text);
                if had_errors {
                    return Err(unsupported(format!(
                        "choice text cannot be represented in Shift-JIS: {text:?}"
                    )));
                }
                // Sunshine's TTalk2D2::setTagParam clamps snprintf to its
                // 0x11-byte choice buffer, leaving at most 16 content bytes
                // before the trailing NUL. Reject text the game would truncate.
                if encoded.len() > SMS_BMG_CHOICE_TEXT_MAX_SHIFT_JIS_BYTES {
                    return Err(unsupported(format!(
                        "choice text uses {} Shift-JIS bytes; Sunshine supports at most {}",
                        encoded.len(),
                        SMS_BMG_CHOICE_TEXT_MAX_SHIFT_JIS_BYTES
                    )));
                }
                let mut payload = vec![1, 0, *slot];
                payload.extend_from_slice(encoded.as_ref());
                payload
            }
            Self::DynamicValue(value) => {
                let id = match value {
                    SmsBmgDynamicValue::TimerFlag20003 => 0,
                    SmsBmgDynamicValue::TimerFlag20002 => 1,
                    SmsBmgDynamicValue::RoundedFlag20004 => 2,
                    SmsBmgDynamicValue::BlueCoinTradeRemainder => 3,
                    SmsBmgDynamicValue::TimerFlag20014 => 6,
                };
                vec![2, 0, id]
            }
            Self::FruitBasketRemaining(basket) => {
                if *basket > 3 {
                    return Err(unsupported(format!(
                        "SMS fruit-basket control index {basket} is outside 0..=3"
                    )));
                }
                vec![2, 0, 4, *basket]
            }
            Self::Color(color) => {
                if *color >= 6 {
                    return Err(unsupported(format!(
                        "SMS text color {color} is outside TTalk2D2's six-color table"
                    )));
                }
                vec![0xFF, 0, 0, *color]
            }
            Self::Unknown(payload) => {
                if payload.len() < 3 {
                    return Err(unsupported(format!(
                        "unknown SMS control payload has {} bytes; expected at least type + id",
                        payload.len()
                    )));
                }
                payload.clone()
            }
        };
        if payload.len() > u8::MAX as usize - 2 {
            return Err(resource_limit(
                "control bytes",
                payload.len() + 2,
                u8::MAX as usize,
            ));
        }
        Ok(payload)
    }
}

impl BmgMessage {
    pub fn encoded_len(&self) -> Result<usize> {
        encode_message(self).map(|bytes| bytes.len())
    }

    pub fn validate_sms_controls(&self) -> Result<()> {
        for token in &self.tokens {
            if let BmgMessageToken::Control(payload) = token {
                SmsBmgControl::decode(payload)?;
            } else if let BmgMessageToken::Text(text) = token {
                let (_, _, had_errors) = SHIFT_JIS.encode(text);
                if had_errors {
                    return Err(unsupported(format!(
                        "message text cannot be represented in Shift-JIS: {text:?}"
                    )));
                }
            }
        }
        Ok(())
    }
}

impl BmgEntry {
    pub fn sms_voice_index(&self) -> Result<u8> {
        self.attributes.get(4).copied().ok_or_else(|| {
            unsupported(format!(
                "BMG entry has {} attribute bytes; SMS voice is attribute byte 4",
                self.attributes.len()
            ))
        })
    }

    pub fn set_sms_voice_index(&mut self, voice_index: u8) -> Result<()> {
        if voice_index as usize >= SMS_TALK_SOUND_LIMIT {
            return Err(resource_limit(
                "talk sound index",
                voice_index as usize,
                SMS_TALK_SOUND_LIMIT - 1,
            ));
        }
        let attribute_len = self.attributes.len();
        let voice = self.attributes.get_mut(4).ok_or_else(|| {
            unsupported(format!(
                "BMG entry has {attribute_len} attribute bytes; SMS voice is attribute byte 4"
            ))
        })?;
        *voice = voice_index;
        Ok(())
    }
}

impl BmgFile {
    pub fn parse(bytes: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = bytes.as_ref();
        require_len(FORMAT, bytes, HEADER_SIZE)?;
        require_magic(FORMAT, bytes, b"MESGbmg1")?;
        let size_units = be_u32(bytes, 0x08, FORMAT)? as usize;
        let declared_size = size_units
            .checked_mul(FILE_ALIGNMENT)
            .ok_or_else(|| invalid_offset(usize::MAX, bytes.len()))?;
        if declared_size != bytes.len() {
            return Err(FormatError::Unsupported {
                format: FORMAT,
                message: format!(
                    "declared size {declared_size:#x} does not equal supplied size {:#x}",
                    bytes.len()
                ),
            });
        }
        let section_count = be_u32(bytes, 0x0C, FORMAT)? as usize;
        if section_count != 2 {
            return Err(unsupported(format!(
                "source-free BMG writer currently requires INF1 + DAT1, found {section_count} sections"
            )));
        }
        let mut header_reserved = [0; 16];
        header_reserved.copy_from_slice(&bytes[0x10..0x20]);

        let info_start = HEADER_SIZE;
        require_magic_at(bytes, info_start, b"INF1")?;
        let info_section_size = be_u32(bytes, info_start + 4, FORMAT)?;
        let info_end = checked_end(info_start, info_section_size as usize, bytes.len())?;
        checked_slice(FORMAT, bytes, info_start, INFO_HEADER_SIZE)?;
        let message_count = be_u16(bytes, info_start + 8, FORMAT)? as usize;
        if message_count > MAX_MESSAGES {
            return Err(resource_limit("messages", message_count, MAX_MESSAGES));
        }
        let entry_size = be_u16(bytes, info_start + 0x0A, FORMAT)?;
        if entry_size < 4 {
            return Err(unsupported(format!(
                "INF1 entry size {entry_size} is smaller than its message offset"
            )));
        }
        let group_id = be_u16(bytes, info_start + 0x0C, FORMAT)?;
        let default_color = bytes[info_start + 0x0E];
        let info_reserved = bytes[info_start + 0x0F];
        let entries_size = message_count
            .checked_mul(entry_size as usize)
            .ok_or_else(|| invalid_offset(usize::MAX, info_end))?;
        let entries_end = checked_end(info_start + INFO_HEADER_SIZE, entries_size, info_end)?;
        if bytes[entries_end..info_end].iter().any(|byte| *byte != 0) {
            return Err(unsupported(
                "INF1 alignment contains non-zero unmodeled bytes".to_string(),
            ));
        }

        let data_start = info_end;
        require_magic_at(bytes, data_start, b"DAT1")?;
        let data_section_size = be_u32(bytes, data_start + 4, FORMAT)?;
        let data_end = checked_end(data_start, data_section_size as usize, bytes.len())?;
        if data_end != bytes.len() {
            return Err(unsupported(format!(
                "{} bytes follow DAT1",
                bytes.len() - data_end
            )));
        }
        let data = checked_slice(
            FORMAT,
            bytes,
            data_start + DATA_HEADER_SIZE,
            data_section_size as usize - DATA_HEADER_SIZE,
        )?;
        let mut claimed = vec![false; data.len()];

        let mut entries = Vec::with_capacity(message_count);
        let mut decoded_by_offset = BTreeMap::<u32, (BmgMessage, usize)>::new();
        for index in 0..message_count {
            let entry = info_start + INFO_HEADER_SIZE + index * entry_size as usize;
            let message_offset = be_u32(bytes, entry, FORMAT)?;
            let (message, consumed) = if let Some(cached) = decoded_by_offset.get(&message_offset) {
                cached.clone()
            } else {
                let decoded = parse_message(data, message_offset as usize)?;
                decoded_by_offset.insert(message_offset, decoded.clone());
                decoded
            };
            let start = message_offset as usize;
            let end = start
                .checked_add(consumed)
                .ok_or_else(|| invalid_offset(start, data.len()))?;
            let span = claimed
                .get_mut(start..end)
                .ok_or_else(|| invalid_offset(end, data.len()))?;
            span.fill(true);
            entries.push(BmgEntry {
                message_offset,
                attributes: bytes[entry + 4..entry + entry_size as usize].to_vec(),
                message,
            });
        }
        if data
            .iter()
            .zip(&claimed)
            .any(|(byte, claimed)| !claimed && *byte != 0)
        {
            return Err(unsupported(
                "DAT1 has non-zero bytes outside typed messages".to_string(),
            ));
        }

        Ok(Self {
            header_reserved,
            info_section_size,
            data_section_size,
            entry_size,
            group_id,
            default_color,
            info_reserved,
            entries,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate_layout()?;
        let info_size = self.info_section_size as usize;
        let data_size = self.data_section_size as usize;
        let file_size = HEADER_SIZE
            .checked_add(info_size)
            .and_then(|size| size.checked_add(data_size))
            .ok_or_else(|| invalid_offset(usize::MAX, usize::MAX))?;
        if !file_size.is_multiple_of(FILE_ALIGNMENT) {
            return Err(unsupported(format!(
                "encoded BMG size {file_size:#x} is not {FILE_ALIGNMENT:#x}-byte aligned"
            )));
        }
        let size_units = u32::try_from(file_size / FILE_ALIGNMENT)
            .map_err(|_| resource_limit("file size units", file_size, u32::MAX as usize))?;
        let mut bytes = vec![0; file_size];
        bytes[..8].copy_from_slice(b"MESGbmg1");
        put_u32(&mut bytes, 0x08, size_units)?;
        put_u32(&mut bytes, 0x0C, 2)?;
        bytes[0x10..0x20].copy_from_slice(&self.header_reserved);

        let info = HEADER_SIZE;
        bytes[info..info + 4].copy_from_slice(b"INF1");
        put_u32(&mut bytes, info + 4, self.info_section_size)?;
        put_u16(&mut bytes, info + 8, self.entries.len() as u16)?;
        put_u16(&mut bytes, info + 0x0A, self.entry_size)?;
        put_u16(&mut bytes, info + 0x0C, self.group_id)?;
        bytes[info + 0x0E] = self.default_color;
        bytes[info + 0x0F] = self.info_reserved;
        for (index, entry) in self.entries.iter().enumerate() {
            let offset = info + INFO_HEADER_SIZE + index * self.entry_size as usize;
            put_u32(&mut bytes, offset, entry.message_offset)?;
            bytes[offset + 4..offset + self.entry_size as usize].copy_from_slice(&entry.attributes);
        }

        let data_start = info + info_size;
        bytes[data_start..data_start + 4].copy_from_slice(b"DAT1");
        put_u32(&mut bytes, data_start + 4, self.data_section_size)?;
        let payload_start = data_start + DATA_HEADER_SIZE;
        let payload_len = data_size - DATA_HEADER_SIZE;
        let mut emitted = BTreeMap::<u32, Vec<u8>>::new();
        for entry in &self.entries {
            let encoded = encode_message(&entry.message)?;
            if let Some(existing) = emitted.get(&entry.message_offset) {
                if existing != &encoded {
                    return Err(unsupported(format!(
                        "entries sharing DAT1 offset {:#x} contain different messages",
                        entry.message_offset
                    )));
                }
                continue;
            }
            let start = entry.message_offset as usize;
            let end = start
                .checked_add(encoded.len())
                .ok_or_else(|| invalid_offset(start, payload_len))?;
            bytes
                .get_mut(payload_start + start..payload_start + end)
                .ok_or_else(|| invalid_offset(end, payload_len))?
                .copy_from_slice(&encoded);
            emitted.insert(entry.message_offset, encoded);
        }
        Ok(bytes)
    }

    /// Validates the stricter layout and runtime limits used by Sunshine's
    /// dialogue loader rather than the wider BMG container format.
    pub fn validate_sms_dialogue(&self) -> Result<()> {
        self.validate_layout()?;
        if self.entry_size != SMS_BMG_ENTRY_SIZE {
            return Err(unsupported(format!(
                "SMS dialogue requires INF1 entry size {SMS_BMG_ENTRY_SIZE}, found {}",
                self.entry_size
            )));
        }
        if self.entries.len() > SMS_BMG_RUNTIME_MESSAGE_LIMIT {
            return Err(resource_limit(
                "runtime messages",
                self.entries.len(),
                SMS_BMG_RUNTIME_MESSAGE_LIMIT,
            ));
        }
        for (index, entry) in self.entries.iter().enumerate() {
            let voice = entry.sms_voice_index()?;
            if voice as usize >= SMS_TALK_SOUND_LIMIT {
                return Err(unsupported(format!(
                    "entry {index} uses talk sound index {voice}; valid SMS indexes are 0..={} ",
                    SMS_TALK_SOUND_LIMIT - 1
                )));
            }
            entry.message.validate_sms_controls().map_err(|error| {
                unsupported(format!(
                    "entry {index} contains an invalid SMS message: {error}"
                ))
            })?;
        }
        self.validate_message_spans()
    }

    /// Returns every INF1 entry sharing the selected entry's DAT1 offset.
    pub fn message_aliases(&self, entry_index: usize) -> Result<Vec<usize>> {
        let entry = self
            .entries
            .get(entry_index)
            .ok_or_else(|| invalid_offset(entry_index, self.entries.len()))?;
        Ok(self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                (candidate.message_offset == entry.message_offset).then_some(index)
            })
            .collect())
    }

    /// Replaces a non-aliased entry without moving any other message. The
    /// original offset is retained whenever the new bytes fit before the next
    /// occupied message; otherwise the replacement is appended to DAT1.
    pub fn replace_message_stable(
        &mut self,
        entry_index: usize,
        message: BmgMessage,
    ) -> Result<BmgStableEdit> {
        self.validate_sms_edit_base()?;
        message.validate_sms_controls()?;
        let aliases = self.message_aliases(entry_index)?;
        if aliases.len() != 1 {
            return Err(unsupported(format!(
                "entry {entry_index} shares DAT1 offset with entries {aliases:?}; clone it or edit all aliases"
            )));
        }
        let previous_offset = self.entries[entry_index].message_offset;
        let encoded_len = message.encoded_len()?;
        let capacity = self.message_capacity(previous_offset)?;
        if encoded_len <= capacity {
            self.entries[entry_index].message = message;
            return Ok(BmgStableEdit {
                entry_index,
                previous_offset: Some(previous_offset),
                message_offset: previous_offset,
                layout: BmgStableEditLayout::PreservedOffset,
            });
        }
        let message_offset = self.allocate_appended_payload(encoded_len)?;
        self.entries[entry_index].message_offset = message_offset;
        self.entries[entry_index].message = message;
        Ok(BmgStableEdit {
            entry_index,
            previous_offset: Some(previous_offset),
            message_offset,
            layout: BmgStableEditLayout::AppendedPayload,
        })
    }

    /// Replaces all entries intentionally sharing one DAT1 message.
    pub fn replace_message_aliases_stable(
        &mut self,
        entry_index: usize,
        message: BmgMessage,
    ) -> Result<Vec<BmgStableEdit>> {
        self.validate_sms_edit_base()?;
        message.validate_sms_controls()?;
        let aliases = self.message_aliases(entry_index)?;
        let previous_offset = self.entries[entry_index].message_offset;
        let encoded_len = message.encoded_len()?;
        let (message_offset, layout) = if encoded_len <= self.message_capacity(previous_offset)? {
            (previous_offset, BmgStableEditLayout::PreservedOffset)
        } else {
            (
                self.allocate_appended_payload(encoded_len)?,
                BmgStableEditLayout::AppendedPayload,
            )
        };
        for alias in &aliases {
            self.entries[*alias].message_offset = message_offset;
            self.entries[*alias].message = message.clone();
        }
        Ok(aliases
            .into_iter()
            .map(|entry_index| BmgStableEdit {
                entry_index,
                previous_offset: Some(previous_offset),
                message_offset,
                layout,
            })
            .collect())
    }

    /// Appends a copy-on-write clone of an entry. The cloned message always
    /// receives its own DAT1 payload even when its bytes match another entry.
    pub fn clone_entry_stable(
        &mut self,
        source_index: usize,
        replacement: Option<BmgMessage>,
    ) -> Result<BmgStableEdit> {
        let source = self
            .entries
            .get(source_index)
            .cloned()
            .ok_or_else(|| invalid_offset(source_index, self.entries.len()))?;
        self.append_entry_stable(source.attributes, replacement.unwrap_or(source.message))
    }

    /// Appends a new INF1 entry and distinct DAT1 payload without reordering or
    /// deduplicating any existing entries.
    pub fn append_entry_stable(
        &mut self,
        attributes: Vec<u8>,
        message: BmgMessage,
    ) -> Result<BmgStableEdit> {
        self.validate_sms_edit_base()?;
        if self.entries.len() >= SMS_BMG_RUNTIME_MESSAGE_LIMIT {
            return Err(resource_limit(
                "runtime messages",
                self.entries.len() + 1,
                SMS_BMG_RUNTIME_MESSAGE_LIMIT,
            ));
        }
        let expected_attributes = self.entry_size as usize - 4;
        if attributes.len() != expected_attributes {
            return Err(unsupported(format!(
                "new BMG entry has {} attribute bytes; expected {expected_attributes}",
                attributes.len()
            )));
        }
        message.validate_sms_controls()?;
        let voice = attributes[4];
        if voice as usize >= SMS_TALK_SOUND_LIMIT {
            return Err(resource_limit(
                "talk sound index",
                voice as usize,
                SMS_TALK_SOUND_LIMIT - 1,
            ));
        }
        let message_offset = self.allocate_appended_payload(message.encoded_len()?)?;
        let entry_index = self.entries.len();
        self.entries.push(BmgEntry {
            message_offset,
            attributes,
            message,
        });
        let info_required = INFO_HEADER_SIZE
            .checked_add(self.entries.len() * self.entry_size as usize)
            .ok_or_else(|| resource_limit("INF1 bytes", usize::MAX, u32::MAX as usize))?;
        if info_required > self.info_section_size as usize {
            self.info_section_size =
                usize_u32(align_up(info_required, FILE_ALIGNMENT)?, "INF1 bytes")?;
        }
        Ok(BmgStableEdit {
            entry_index,
            previous_offset: None,
            message_offset,
            layout: BmgStableEditLayout::AppendedPayload,
        })
    }

    /// Packs edited messages contiguously and recomputes INF1/DAT1 sizes.
    pub fn canonicalize_layout(&mut self) -> Result<()> {
        let mut cursor = usize::from(self.entries.iter().all(|entry| entry.message_offset != 0));
        let mut offsets = BTreeMap::<BmgMessage, u32>::new();
        for entry in &mut self.entries {
            if let Some(offset) = offsets.get(&entry.message) {
                entry.message_offset = *offset;
                continue;
            }
            let encoded = encode_message(&entry.message)?;
            let offset = u32::try_from(cursor)
                .map_err(|_| resource_limit("DAT1 bytes", cursor, u32::MAX as usize))?;
            entry.message_offset = offset;
            offsets.insert(entry.message.clone(), offset);
            cursor = cursor
                .checked_add(encoded.len())
                .ok_or_else(|| resource_limit("DAT1 bytes", usize::MAX, u32::MAX as usize))?;
        }
        let info_used = INFO_HEADER_SIZE + self.entries.len() * self.entry_size as usize;
        self.info_section_size = align_up(info_used, FILE_ALIGNMENT)? as u32;
        self.data_section_size = align_up(DATA_HEADER_SIZE + cursor, FILE_ALIGNMENT)? as u32;
        Ok(())
    }

    fn validate_layout(&self) -> Result<()> {
        if self.entries.len() > MAX_MESSAGES {
            return Err(resource_limit("messages", self.entries.len(), MAX_MESSAGES));
        }
        if self.entry_size < 4 {
            return Err(unsupported("INF1 entry size is smaller than 4".to_string()));
        }
        let attributes = self.entry_size as usize - 4;
        if let Some((index, entry)) = self
            .entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.attributes.len() != attributes)
        {
            return Err(unsupported(format!(
                "entry {index} has {} attribute bytes; expected {attributes}",
                entry.attributes.len()
            )));
        }
        let info_required = INFO_HEADER_SIZE + self.entries.len() * self.entry_size as usize;
        if (self.info_section_size as usize) < info_required {
            return Err(invalid_offset(
                info_required,
                self.info_section_size as usize,
            ));
        }
        if self.data_section_size < DATA_HEADER_SIZE as u32 {
            return Err(invalid_offset(
                DATA_HEADER_SIZE,
                self.data_section_size as usize,
            ));
        }
        Ok(())
    }

    fn validate_sms_edit_base(&self) -> Result<()> {
        self.validate_layout()?;
        if self.entry_size != SMS_BMG_ENTRY_SIZE {
            return Err(unsupported(format!(
                "stable SMS dialogue editing requires entry size {SMS_BMG_ENTRY_SIZE}, found {}",
                self.entry_size
            )));
        }
        if self.entries.len() > SMS_BMG_RUNTIME_MESSAGE_LIMIT {
            return Err(resource_limit(
                "runtime messages",
                self.entries.len(),
                SMS_BMG_RUNTIME_MESSAGE_LIMIT,
            ));
        }
        self.validate_message_spans()
    }

    fn message_capacity(&self, message_offset: u32) -> Result<usize> {
        let payload_len = self.data_section_size as usize - DATA_HEADER_SIZE;
        let next = self
            .entries
            .iter()
            .map(|entry| entry.message_offset)
            .filter(|offset| *offset > message_offset)
            .min()
            .map_or(payload_len, |offset| offset as usize);
        next.checked_sub(message_offset as usize)
            .ok_or_else(|| invalid_offset(message_offset as usize, payload_len))
    }

    fn occupied_payload_end(&self) -> Result<usize> {
        self.entries.iter().try_fold(0usize, |end, entry| {
            let entry_end = (entry.message_offset as usize)
                .checked_add(entry.message.encoded_len()?)
                .ok_or_else(|| resource_limit("DAT1 bytes", usize::MAX, u32::MAX as usize))?;
            Ok(end.max(entry_end))
        })
    }

    fn allocate_appended_payload(&mut self, encoded_len: usize) -> Result<u32> {
        let offset = self.occupied_payload_end()?;
        let end = offset
            .checked_add(encoded_len)
            .ok_or_else(|| resource_limit("DAT1 bytes", usize::MAX, u32::MAX as usize))?;
        let required = DATA_HEADER_SIZE
            .checked_add(end)
            .ok_or_else(|| resource_limit("DAT1 bytes", usize::MAX, u32::MAX as usize))?;
        if required > self.data_section_size as usize {
            self.data_section_size = usize_u32(align_up(required, FILE_ALIGNMENT)?, "DAT1 bytes")?;
        }
        usize_u32(offset, "DAT1 message offset")
    }

    fn validate_message_spans(&self) -> Result<()> {
        let payload_len = self.data_section_size as usize - DATA_HEADER_SIZE;
        let mut spans = BTreeMap::<u32, (usize, Vec<u8>)>::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let encoded = encode_message(&entry.message)?;
            let encoded_len = encoded.len();
            let end = (entry.message_offset as usize)
                .checked_add(encoded_len)
                .ok_or_else(|| invalid_offset(entry.message_offset as usize, payload_len))?;
            if end > payload_len {
                return Err(unsupported(format!(
                    "entry {index} message ends at {end:#x}, outside DAT1 payload {payload_len:#x}"
                )));
            }
            if let Some((existing_len, existing)) = spans.get(&entry.message_offset) {
                if *existing_len != encoded_len || existing != &encoded {
                    return Err(unsupported(format!(
                        "entries sharing DAT1 offset {:#x} have different encoded messages",
                        entry.message_offset
                    )));
                }
                continue;
            }
            spans.insert(entry.message_offset, (encoded_len, encoded));
        }
        let sorted = spans.into_iter().collect::<Vec<_>>();
        for pair in sorted.windows(2) {
            let (offset, (len, _)) = &pair[0];
            let (next_offset, _) = &pair[1];
            let end = *offset as usize + *len;
            if end > *next_offset as usize {
                return Err(unsupported(format!(
                    "DAT1 message {offset:#x}..{end:#x} overlaps message at {next_offset:#x}"
                )));
            }
        }
        Ok(())
    }
}

fn parse_message(data: &[u8], start: usize) -> Result<(BmgMessage, usize)> {
    if start >= data.len() {
        return Err(invalid_offset(start, data.len()));
    }
    let mut tokens = Vec::new();
    let mut cursor = start;
    let mut text_start = start;
    while cursor < data.len() {
        match data[cursor] {
            0 => {
                push_text_token(&mut tokens, &data[text_start..cursor])?;
                return Ok((BmgMessage { tokens }, cursor - start + 1));
            }
            0x1A => {
                push_text_token(&mut tokens, &data[text_start..cursor])?;
                let length = *data
                    .get(cursor + 1)
                    .ok_or_else(|| invalid_offset(cursor + 1, data.len()))?
                    as usize;
                if length < 2 {
                    return Err(unsupported(format!(
                        "BMG control at {cursor:#x} has invalid length {length}"
                    )));
                }
                let end = cursor
                    .checked_add(length)
                    .ok_or_else(|| invalid_offset(cursor, data.len()))?;
                let control = data
                    .get(cursor + 2..end)
                    .ok_or_else(|| invalid_offset(end, data.len()))?;
                tokens.push(BmgMessageToken::Control(control.to_vec()));
                cursor = end;
                text_start = cursor;
            }
            _ => cursor += 1,
        }
    }
    Err(invalid_offset(cursor, data.len()))
}

fn push_text_token(tokens: &mut Vec<BmgMessageToken>, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let text = SHIFT_JIS
        .decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or_else(|| unsupported("message contains invalid Shift-JIS text".to_string()))?;
    tokens.push(BmgMessageToken::Text(text.into_owned()));
    Ok(())
}

fn encode_message(message: &BmgMessage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for token in &message.tokens {
        match token {
            BmgMessageToken::Text(text) => {
                let (encoded, _, had_errors) = SHIFT_JIS.encode(text);
                if had_errors {
                    return Err(unsupported(format!(
                        "message text cannot be represented in Shift-JIS: {text:?}"
                    )));
                }
                bytes.extend_from_slice(&encoded);
            }
            BmgMessageToken::Control(payload) => {
                let length = payload
                    .len()
                    .checked_add(2)
                    .ok_or_else(|| resource_limit("control bytes", usize::MAX, u8::MAX as usize))?;
                let length = u8::try_from(length)
                    .map_err(|_| resource_limit("control bytes", length, u8::MAX as usize))?;
                bytes.push(0x1A);
                bytes.push(length);
                bytes.extend_from_slice(payload);
            }
        }
    }
    bytes.push(0);
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(resource_limit(
            "message bytes",
            bytes.len(),
            MAX_MESSAGE_BYTES,
        ));
    }
    Ok(bytes)
}

fn require_magic_at(bytes: &[u8], offset: usize, expected: &'static [u8]) -> Result<()> {
    let actual = checked_slice(FORMAT, bytes, offset, expected.len())?;
    if actual != expected {
        return Err(FormatError::BadMagic {
            format: FORMAT,
            expected,
            actual: actual.to_vec(),
        });
    }
    Ok(())
}

fn checked_end(start: usize, length: usize, limit: usize) -> Result<usize> {
    let end = start
        .checked_add(length)
        .ok_or_else(|| invalid_offset(start, limit))?;
    if end > limit {
        return Err(invalid_offset(end, limit));
    }
    Ok(end)
}

fn align_up(value: usize, alignment: usize) -> Result<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value / alignment * alignment)
        .ok_or_else(|| resource_limit("aligned bytes", usize::MAX, u32::MAX as usize))
}

fn usize_u32(value: usize, resource: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| resource_limit(resource, value, u32::MAX as usize))
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) -> Result<()> {
    let len = bytes.len();
    bytes
        .get_mut(offset..offset + 2)
        .ok_or_else(|| invalid_offset(offset, len))?
        .copy_from_slice(&value.to_be_bytes());
    Ok(())
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<()> {
    let len = bytes.len();
    bytes
        .get_mut(offset..offset + 4)
        .ok_or_else(|| invalid_offset(offset, len))?
        .copy_from_slice(&value.to_be_bytes());
    Ok(())
}

fn invalid_offset(offset: usize, len: usize) -> FormatError {
    FormatError::InvalidOffset {
        format: FORMAT,
        offset,
        len,
    }
}

fn unsupported(message: String) -> FormatError {
    FormatError::Unsupported {
        format: FORMAT,
        message,
    }
}

fn resource_limit(resource: &'static str, requested: usize, limit: usize) -> FormatError {
    FormatError::ResourceLimit {
        format: FORMAT,
        resource,
        requested,
        limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> BmgMessage {
        BmgMessage {
            tokens: vec![BmgMessageToken::Text(value.to_string())],
        }
    }

    fn fixture(messages: &[&str]) -> BmgFile {
        let mut file = BmgFile {
            header_reserved: [0xCD; 16],
            info_section_size: 0,
            data_section_size: 0,
            entry_size: SMS_BMG_ENTRY_SIZE,
            group_id: 7,
            default_color: 1,
            info_reserved: 0,
            entries: messages
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let mut attributes = vec![0; 8];
                    attributes[4] = index as u8;
                    BmgEntry {
                        message_offset: 0,
                        attributes,
                        message: text(value),
                    }
                })
                .collect(),
        };
        file.canonicalize_layout().unwrap();
        file
    }

    #[test]
    fn sms_controls_are_typed_and_unknown_controls_remain_raw() {
        let controls = [
            SmsBmgControl::CharacterDelay(12),
            SmsBmgControl::AutomaticContinuation,
            SmsBmgControl::Choice {
                slot: 1,
                text: "はい".to_string(),
            },
            SmsBmgControl::DynamicValue(SmsBmgDynamicValue::TimerFlag20014),
            SmsBmgControl::FruitBasketRemaining(3),
            SmsBmgControl::Color(5),
            SmsBmgControl::Unknown(vec![0x7E, 0x12, 0x34, 0xAA]),
        ];
        for control in controls {
            let payload = control.encode_payload().unwrap();
            assert_eq!(SmsBmgControl::decode(&payload).unwrap(), control);
            let token = BmgMessageToken::from_sms_control(control.clone()).unwrap();
            assert_eq!(token.sms_control().unwrap(), Some(control));
        }
        assert!(SmsBmgControl::Color(6).encode_payload().is_err());
        assert!(SmsBmgControl::FruitBasketRemaining(4)
            .encode_payload()
            .is_err());
        assert!(SmsBmgControl::decode(&[0, 0, 0]).is_err());
    }

    #[test]
    fn sms_choice_text_enforces_sunshine_shift_jis_byte_limit() {
        let sixteen_single_byte = SmsBmgControl::Choice {
            slot: 0,
            text: "A".repeat(16),
        };
        assert_eq!(sixteen_single_byte.encode_payload().unwrap().len(), 3 + 16);

        let seventeen_single_byte = SmsBmgControl::Choice {
            slot: 0,
            text: "A".repeat(17),
        };
        assert!(seventeen_single_byte.encode_payload().is_err());

        let sixteen_multibyte = SmsBmgControl::Choice {
            slot: 1,
            text: "\u{3042}".repeat(8),
        };
        assert_eq!(sixteen_multibyte.encode_payload().unwrap().len(), 3 + 16);

        let seventeen_mixed_bytes = SmsBmgControl::Choice {
            slot: 1,
            text: format!("{}A", "\u{3042}".repeat(8)),
        };
        assert!(seventeen_mixed_bytes.encode_payload().is_err());
    }

    #[test]
    fn stable_replacement_preserves_offsets_or_appends_without_reordering() {
        let mut file = fixture(&["short", "untouched"]);
        let second_offset = file.entries[1].message_offset;
        let original_second = file.entries[1].clone();
        let first_offset = file.entries[0].message_offset;
        let edit = file
            .replace_message_stable(0, text("tiny"))
            .expect("short replacement");
        assert_eq!(edit.layout, BmgStableEditLayout::PreservedOffset);
        assert_eq!(edit.message_offset, first_offset);
        assert_eq!(file.entries[1], original_second);

        let long = "a".repeat(200);
        let edit = file
            .replace_message_stable(0, text(&long))
            .expect("growing replacement");
        assert_eq!(edit.layout, BmgStableEditLayout::AppendedPayload);
        assert!(edit.message_offset > second_offset);
        assert_eq!(file.entries[1], original_second);
        let encoded = file.encode().unwrap();
        let reparsed = BmgFile::parse(&encoded).unwrap();
        assert_eq!(reparsed, file);
        assert_eq!(reparsed.encode().unwrap(), encoded);
    }

    #[test]
    fn stable_clone_splits_alias_and_preserves_attributes() {
        let mut file = fixture(&["shared", "shared"]);
        assert_eq!(file.message_aliases(0).unwrap(), vec![0, 1]);
        let source_attributes = file.entries[0].attributes.clone();
        let clone = file
            .clone_entry_stable(0, Some(text("instance only")))
            .unwrap();
        assert_eq!(clone.entry_index, 2);
        assert_eq!(clone.layout, BmgStableEditLayout::AppendedPayload);
        assert_ne!(clone.message_offset, file.entries[0].message_offset);
        assert_eq!(file.entries[2].attributes, source_attributes);
        assert_eq!(file.message_aliases(0).unwrap(), vec![0, 1]);
        assert_eq!(file.message_aliases(2).unwrap(), vec![2]);

        let edits = file
            .replace_message_aliases_stable(0, text("all users"))
            .unwrap();
        assert_eq!(edits.len(), 2);
        assert_eq!(file.entries[0].message, file.entries[1].message);
        assert_ne!(file.entries[0].message, file.entries[2].message);
        file.validate_sms_dialogue().unwrap();
    }

    #[test]
    fn sms_runtime_limits_are_enforced() {
        let mut file = fixture(&["voice"]);
        file.entries[0].set_sms_voice_index(134).unwrap();
        assert_eq!(file.entries[0].sms_voice_index().unwrap(), 134);
        assert!(file.entries[0].set_sms_voice_index(135).is_err());

        let template = file.entries[0].clone();
        while file.entries.len() < SMS_BMG_RUNTIME_MESSAGE_LIMIT {
            file.append_entry_stable(template.attributes.clone(), text("x"))
                .unwrap();
        }
        assert!(file
            .append_entry_stable(template.attributes, text("overflow"))
            .is_err());
        file.validate_sms_dialogue().unwrap();
    }

    #[test]
    fn extracted_stage_message_rebuilds_when_fixture_exists() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".codex_scratch/message.bmg");
        if !path.is_file() {
            return;
        }
        let source = std::fs::read(path).unwrap();
        let file = BmgFile::parse(&source).unwrap();
        assert_eq!(file.entries.len(), 31);
        assert_eq!(file.encode().unwrap(), source);
    }

    #[test]
    #[ignore = "requires SMS_BASE_ROOT with extracted retail stage archives"]
    fn source_free_rebuilds_every_retail_stage_bmg_file() {
        let root = std::env::var_os("SMS_BASE_ROOT")
            .map(std::path::PathBuf::from)
            .expect("set SMS_BASE_ROOT to an extracted retail game root");
        let archives = crate::discover_scene_archives(root).expect("discover stage archives");
        let mut rebuilt = 0usize;
        for archive in archives {
            for asset in crate::mount_scene_archive(&archive.path)
                .unwrap_or_else(|error| panic!("mount {}: {error}", archive.path.display()))
            {
                if !asset
                    .path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(".bmg")
                {
                    continue;
                }
                let source = crate::read_stage_asset_bytes(&asset.path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", asset.path.display()));
                let document = BmgFile::parse(&source)
                    .unwrap_or_else(|error| panic!("parse {}: {error}", asset.path.display()));
                if document.entry_size == SMS_BMG_ENTRY_SIZE
                    && document.entries.len() <= SMS_BMG_RUNTIME_MESSAGE_LIMIT
                {
                    document.validate_sms_dialogue().unwrap_or_else(|error| {
                        panic!("validate SMS dialogue {}: {error}", asset.path.display())
                    });
                }
                assert_eq!(
                    document.encode().expect("encode semantic BMG"),
                    source,
                    "source-free BMG rebuild differs for {}",
                    asset.path.display()
                );
                rebuilt += 1;
            }
        }
        assert!(rebuilt > 0, "retail census found no BMG files");
        eprintln!("source-free BMG census rebuilt {rebuilt} files");
    }

    #[test]
    #[ignore = "requires SMS_BASE_ROOT with an extracted retail common.szs"]
    fn source_free_rebuilds_retail_common_bmg_files() {
        let root = std::env::var_os("SMS_BASE_ROOT")
            .map(std::path::PathBuf::from)
            .expect("set SMS_BASE_ROOT to an extracted retail game root");
        let candidates = [
            root.join("files/data/common.szs"),
            root.join("data/common.szs"),
            root.join("common.szs"),
        ];
        let common = candidates
            .into_iter()
            .find(|path| path.is_file())
            .expect("locate common.szs below SMS_BASE_ROOT");
        let mut rebuilt = 0usize;
        let mut dialogue = 0usize;
        for asset in crate::mount_scene_archive(&common).expect("mount common.szs") {
            let lower = asset.path.to_string_lossy().to_ascii_lowercase();
            if !lower.ends_with(".bmg") {
                continue;
            }
            let source = crate::read_stage_asset_bytes(&asset.path)
                .unwrap_or_else(|error| panic!("read {}: {error}", asset.path.display()));
            let document = BmgFile::parse(&source)
                .unwrap_or_else(|error| panic!("parse {}: {error}", asset.path.display()));
            if lower.ends_with("/2d/sys_message.bmg") || lower.ends_with("/2d/balloon.bmg") {
                document.validate_sms_dialogue().unwrap_or_else(|error| {
                    panic!("validate SMS dialogue {}: {error}", asset.path.display())
                });
                dialogue += 1;
            }
            assert_eq!(
                document.encode().expect("encode semantic common BMG"),
                source,
                "source-free common BMG rebuild differs for {}",
                asset.path.display()
            );
            rebuilt += 1;
        }
        assert!(rebuilt > 0, "retail common.szs contains no BMG files");
        assert_eq!(dialogue, 2, "expected system and balloon dialogue BMGs");
        eprintln!("source-free common BMG census rebuilt {rebuilt} files");
    }
}
