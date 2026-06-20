use std::collections::HashSet;

pub const NON_HP_ALIAS_CONTEXT_KEYS: [&str; 6] = [
    "actor_channel",
    "iris_ref32",
    "netguid32",
    "netguid_packed",
    "sdk_net_target",
    "target_stream",
];

pub fn target_context_value<'a>(context: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    context
        .iter()
        .find_map(|value| value.strip_prefix(&prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "None")
}

pub fn target_context_values<'a>(
    context: &'a [String],
    key: &'a str,
) -> impl Iterator<Item = &'a str> {
    let prefix = format!("{key}=");
    context
        .iter()
        .filter_map(move |value| value.strip_prefix(&prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "None")
}

pub fn target_alias_lookup_keys(target_id: Option<&String>, context: &[String]) -> HashSet<String> {
    let mut keys = HashSet::new();
    if let Some(target_id) = target_id {
        extend_equivalent_target_alias_keys(&mut keys, target_id);
    }
    for value in target_context_values(context, "target_handle_candidate") {
        extend_equivalent_target_alias_keys(&mut keys, value);
    }
    for value in target_context_values(context, "boss_hp_guid") {
        extend_equivalent_target_alias_keys(&mut keys, &format!("boss_hp_guid:{value}"));
    }
    for value in target_context_values(context, "current_hp_token") {
        extend_equivalent_target_alias_keys(&mut keys, &format!("current_hp_token:{value}"));
    }
    for key in NON_HP_ALIAS_CONTEXT_KEYS {
        for value in target_context_values(context, key) {
            extend_equivalent_target_alias_keys(&mut keys, &format!("{key}:{value}"));
        }
    }
    for base_handle in target_context_values(context, "base_handle") {
        for slot in target_context_values(context, "sdk_target_slot") {
            extend_equivalent_target_alias_keys(
                &mut keys,
                &format!("target_stream_base_slot:{base_handle}:{slot}"),
            );
        }
        for suffix in target_context_values(context, "sdk_target_suffix") {
            extend_equivalent_target_alias_keys(
                &mut keys,
                &format!("target_stream_base_suffix:{base_handle}:{suffix}"),
            );
        }
    }
    keys
}

pub fn non_hp_alias_keys(target_id: Option<&String>, context: &[String]) -> HashSet<String> {
    target_alias_lookup_keys(target_id, context)
        .into_iter()
        .filter(|key| !is_hp_alias_key(key))
        .collect()
}

pub fn hp_alias_keys(target_id: Option<&String>, context: &[String]) -> HashSet<String> {
    target_alias_lookup_keys(target_id, context)
        .into_iter()
        .filter(|key| is_hp_alias_key(key))
        .collect()
}

pub fn extend_equivalent_target_alias_keys(keys: &mut HashSet<String>, key: &str) {
    for key in equivalent_target_alias_keys(key) {
        keys.insert(key);
    }
}

pub fn equivalent_target_alias_keys(key: &str) -> Vec<String> {
    let key = normalize_target_alias_key(key);
    let mut keys = vec![key.clone()];
    if let Some(value) = key.strip_prefix("AttributeGuid:") {
        keys.push(format!("boss_hp_guid:{value}"));
    } else if let Some(value) = key.strip_prefix("boss_hp_guid:") {
        keys.push(format!("AttributeGuid:{value}"));
    } else if let Some(value) = key.strip_prefix("NetRefHandleCandidate:currenthp:") {
        keys.push(format!("current_hp_token:{value}"));
    } else if let Some(value) = key.strip_prefix("current_hp_token:") {
        keys.push(format!("NetRefHandleCandidate:currenthp:{value}"));
    } else if let Some(value) = key.strip_prefix("NetRefHandleCandidate:sdk_target:") {
        keys.push(format!("sdk_net_target:{value}"));
        keys.extend(sdk_target_token_aliases(value));
    } else if let Some(value) = key.strip_prefix("sdk_net_target:") {
        keys.push(format!("NetRefHandleCandidate:sdk_target:{value}"));
        keys.extend(sdk_target_token_aliases(value));
    } else if let Some(value) = key.strip_prefix("target_stream:") {
        keys.extend(target_stream_id_aliases(value));
    } else if let Some(value) = key.strip_prefix("NetRefHandleCandidate:") {
        keys.push(format!("iris_ref32:{value}"));
    } else if let Some(value) = key.strip_prefix("iris_ref32:") {
        keys.push(format!("NetRefHandleCandidate:{value}"));
    } else if let Some(value) = key.strip_prefix("NetGuidCandidate:") {
        keys.push(format!("netguid32:{value}"));
        keys.push(format!("netguid_packed:{value}"));
    } else if let Some(value) = key.strip_prefix("netguid32:") {
        keys.push(format!("NetGuidCandidate:{value}"));
        keys.push(format!("netguid_packed:{value}"));
    } else if let Some(value) = key.strip_prefix("netguid_packed:") {
        keys.push(format!("NetGuidCandidate:{value}"));
        keys.push(format!("netguid32:{value}"));
    }
    keys
}

fn sdk_target_token_aliases(value: &str) -> Vec<String> {
    let Ok(bytes) = hex::decode(value.trim()) else {
        return Vec::new();
    };
    if bytes.len() < 36 {
        return Vec::new();
    }
    let base_handle = hex::encode(&bytes[16..32]);
    if base_handle.chars().all(|character| character == '0') {
        return Vec::new();
    }
    let suffix = hex::encode(&bytes[32..36]);
    let slot = u32::from_le_bytes(bytes[32..36].try_into().expect("suffix len"));
    let mut aliases = vec![format!("target_stream_base_suffix:{base_handle}:{suffix}")];
    if slot <= 4096 {
        aliases.push(format!("target_stream_base_slot:{base_handle}:{slot}"));
    }
    aliases
}

fn target_stream_id_aliases(value: &str) -> Vec<String> {
    let Some((base_handle, rest)) = value.split_once(":slot:") else {
        return Vec::new();
    };
    let Some((slot, _generation)) = rest.split_once(":gen:") else {
        return Vec::new();
    };
    if slot == "unknown" {
        return Vec::new();
    }
    vec![format!("target_stream_base_slot:{base_handle}:{slot}")]
}

pub fn is_hp_alias_key(key: &str) -> bool {
    key.starts_with("AttributeGuid:")
        || key.starts_with("boss_hp_guid:")
        || key.starts_with("current_hp_token:")
        || key.starts_with("NetRefHandleCandidate:currenthp:")
}

pub fn is_legacy_handle_candidate_id(key: &str) -> bool {
    key.starts_with("AttributeGuid:")
        || key.starts_with("NetRefHandleCandidate:")
        || key.starts_with("NetGuidCandidate:")
}

pub fn normalize_target_alias_key(key: &str) -> String {
    let key = key.trim().split('|').next().unwrap_or(key.trim());
    let Some((kind, value)) = key.split_once(':') else {
        return key.to_owned();
    };
    format!("{kind}:{}", value.trim().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_expand_equivalent_hp_and_non_hp_forms() {
        let mut context = vec![
            "boss_hp_guid=ABCDEF".to_owned(),
            "netguid32=1234".to_owned(),
        ];
        context.push("target_handle_candidate=NetRefHandleCandidate:currenthp:FACE".to_owned());
        let keys =
            target_alias_lookup_keys(Some(&"AttributeGuid:ABCDEF|path=X".to_owned()), &context);
        assert!(keys.contains("AttributeGuid:abcdef"));
        assert!(keys.contains("boss_hp_guid:abcdef"));
        assert!(keys.contains("NetRefHandleCandidate:currenthp:face"));
        assert!(keys.contains("current_hp_token:face"));
        assert!(keys.contains("netguid32:1234"));
        assert!(keys.contains("NetGuidCandidate:1234"));
    }

    #[test]
    fn sdk_target_token_expands_to_stream_base_suffix_aliases() {
        let token =
            "a20000002006068606000000002000005b41c437d248b54e8959424b7501eae40200000000000000";
        let keys = target_alias_lookup_keys(
            Some(&format!("NetRefHandleCandidate:sdk_target:{token}")),
            &[],
        );

        assert!(keys.contains("sdk_net_target:a20000002006068606000000002000005b41c437d248b54e8959424b7501eae40200000000000000"));
        assert!(
            keys.contains("target_stream_base_suffix:5b41c437d248b54e8959424b7501eae4:02000000")
        );
        assert!(keys.contains("target_stream_base_slot:5b41c437d248b54e8959424b7501eae4:2"));
    }

    #[test]
    fn stream_context_expands_to_same_base_suffix_aliases() {
        let context = vec![
            "target_stream=target_stream:5b41c437d248b54e8959424b7501eae4:slot:2:gen:1".to_owned(),
            "base_handle=5b41c437d248b54e8959424b7501eae4".to_owned(),
            "sdk_target_slot=2".to_owned(),
            "sdk_target_suffix=02000000".to_owned(),
        ];
        let keys = target_alias_lookup_keys(None, &context);

        assert!(keys.contains("target_stream_base_slot:5b41c437d248b54e8959424b7501eae4:2"));
        assert!(
            keys.contains("target_stream_base_suffix:5b41c437d248b54e8959424b7501eae4:02000000")
        );
    }
}
