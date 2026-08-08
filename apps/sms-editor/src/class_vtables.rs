//! Class vtable addresses, read from the retail symbol map.
//!
//! A wash that reaches every actor is handed whatever the spray touched, so it
//! has to tell the actors this tool authored from the ones it merely met. An
//! actor carries its class at offset zero, which makes the vtable the cheapest
//! identity available to a stub -- one load and a compare.

use std::collections::BTreeMap;
use std::sync::OnceLock;

const TABLE: &str = include_str!("class_vtables.json");

fn table() -> &'static BTreeMap<String, u32> {
    static TABLE_ONCE: OnceLock<BTreeMap<String, u32>> = OnceLock::new();
    TABLE_ONCE.get_or_init(|| {
        let parsed: serde_json::Value = match serde_json::from_str(TABLE) {
            Ok(parsed) => parsed,
            Err(_) => return BTreeMap::new(),
        };
        parsed["vtables"]
            .as_object()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|(name, value)| {
                        Some((name.clone(), u32::try_from(value.as_u64()?).ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// The vtable a class's instances carry, where the retail image has one.
pub(super) fn class_vtable(class_name: &str) -> Option<u32> {
    table().get(class_name).copied()
}
