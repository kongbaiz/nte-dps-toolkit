//! Stable, frontend-neutral schema for the NTE C++ Mod host API.
//!
//! The Tauri/React editor consumes this schema so completion and signature
//! metadata have one Rust fact source.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModSdkSymbolKind {
    Declaration,
    Snippet,
    Function,
    Property,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModSdkSymbol {
    pub label: &'static str,
    pub insert_text: &'static str,
    pub kind: ModSdkSymbolKind,
    pub return_type: Option<&'static str>,
    pub documentation_key: &'static str,
}

#[derive(Clone, Copy)]
struct ModSdkSymbolSeed {
    label: &'static str,
    insert: &'static str,
}

pub const MOD_SDK_SCHEMA_VERSION: u32 = 2;
pub const MAX_MOD_SDK_SYMBOLS: usize = 128;

pub fn mod_sdk_symbols() -> impl ExactSizeIterator<Item = ModSdkSymbol> {
    MOD_SDK_SYMBOL_SEEDS.iter().copied().map(project_symbol)
}

const MOD_SDK_SYMBOL_SEEDS: &[ModSdkSymbolSeed] = &[
    ModSdkSymbolSeed {
        label: "#include <nte/mod.hpp>",
        insert: "#include <nte/mod.hpp>",
    },
    ModSdkSymbolSeed {
        label: "NTE_SCRIPT(5);",
        insert: "NTE_SCRIPT(5);",
    },
    ModSdkSymbolSeed {
        label: "NTE_MOD(\"id\");",
        insert: "NTE_MOD(\"mod-id\");",
    },
    ModSdkSymbolSeed {
        label: "NTE_REQUIRES(\"capability\");",
        insert: "NTE_REQUIRES(\"viewport.tick\");",
    },
    ModSdkSymbolSeed {
        label: "NTE_BIND(\"binding-id\");",
        insert: "NTE_BIND(\"feature.binding-id\");",
    },
    ModSdkSymbolSeed {
        label: "NTE_ROUTE_IPC(operation, \"kernel.service\");",
        insert: "NTE_ROUTE_IPC(12, \"ipc.query_mod_events\");",
    },
    ModSdkSymbolSeed {
        label: "std::uint64_t state = 0;",
        insert: "std::uint64_t state_name = 0;",
    },
    ModSdkSymbolSeed {
        label: "void on_viewport_tick(const nte::viewport_tick_event& event)",
        insert: "void on_viewport_tick(const nte::viewport_tick_event& event)\n{\n    \n}",
    },
    ModSdkSymbolSeed {
        label: "event.viewport",
        insert: "event.viewport",
    },
    ModSdkSymbolSeed {
        label: "nte::game::viewport",
        insert: "nte::game::viewport",
    },
    ModSdkSymbolSeed {
        label: "nte::game::instance",
        insert: "nte::game::instance",
    },
    ModSdkSymbolSeed {
        label: "nte::game::local_player",
        insert: "nte::game::local_player",
    },
    ModSdkSymbolSeed {
        label: "nte::game::player_controller",
        insert: "nte::game::player_controller",
    },
    ModSdkSymbolSeed {
        label: "nte::game::player_state",
        insert: "nte::game::player_state",
    },
    ModSdkSymbolSeed {
        label: "nte::game::player_character",
        insert: "nte::game::player_character",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::read_ptr(base, offset)",
        insert: "nte::memory::read_ptr(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::read_u8(base, offset)",
        insert: "nte::memory::read_u8(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::read_u16(base, offset)",
        insert: "nte::memory::read_u16(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::read_u32(base, offset)",
        insert: "nte::memory::read_u32(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::read_u64(base, offset)",
        insert: "nte::memory::read_u64(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::read_i32(base, offset)",
        insert: "nte::memory::read_i32(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::read_f32_milli(base, offset)",
        insert: "nte::memory::read_f32_milli(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::read_fname_hash(base, offset)",
        insert: "nte::memory::read_fname_hash(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::tarray_first(base, offset)",
        insert: "nte::memory::tarray_first(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::tarray_count(base, offset)",
        insert: "nte::memory::tarray_count(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::is_readable(pointer, size)",
        insert: "nte::memory::is_readable(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::write_u8(base, offset, value)",
        insert: "nte::memory::write_u8(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::write_u16(base, offset, value)",
        insert: "nte::memory::write_u16(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::write_u32(base, offset, value)",
        insert: "nte::memory::write_u32(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::write_u64(base, offset, value)",
        insert: "nte::memory::write_u64(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::write_i32(base, offset, value)",
        insert: "nte::memory::write_i32(",
    },
    ModSdkSymbolSeed {
        label: "nte::memory::write_f32_milli(base, offset, value)",
        insert: "nte::memory::write_f32_milli(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::find_function(object, \"Owner\", \"Function\")",
        insert: "nte::unreal::find_function(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_clear(size)",
        insert: "nte::unreal::params_clear(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_write_u8(offset, value)",
        insert: "nte::unreal::params_write_u8(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_write_u16(offset, value)",
        insert: "nte::unreal::params_write_u16(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_write_u32(offset, value)",
        insert: "nte::unreal::params_write_u32(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_write_u64(offset, value)",
        insert: "nte::unreal::params_write_u64(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_write_i32(offset, value)",
        insert: "nte::unreal::params_write_i32(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_write_f32_milli(offset, value)",
        insert: "nte::unreal::params_write_f32_milli(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_read_u8(offset)",
        insert: "nte::unreal::params_read_u8(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_read_u16(offset)",
        insert: "nte::unreal::params_read_u16(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_read_u32(offset)",
        insert: "nte::unreal::params_read_u32(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_read_u64(offset)",
        insert: "nte::unreal::params_read_u64(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_read_i32(offset)",
        insert: "nte::unreal::params_read_i32(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::params_read_f32_milli(offset)",
        insert: "nte::unreal::params_read_f32_milli(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::call(object, function)",
        insert: "nte::unreal::call(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::watch(object, function)",
        insert: "nte::unreal::watch(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::watch_array_u64(object, function, element_size, value_offset)",
        insert: "nte::unreal::watch_array_u64(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::watch_class_array_u64(object, function, element_size, value_offset)",
        insert: "nte::unreal::watch_class_array_u64(",
    },
    ModSdkSymbolSeed {
        label: "nte::unreal::unwatch(object, function)",
        insert: "nte::unreal::unwatch(",
    },
    ModSdkSymbolSeed {
        label: "nte::event::next()",
        insert: "nte::event::next()",
    },
    ModSdkSymbolSeed {
        label: "nte::event::object()",
        insert: "nte::event::object()",
    },
    ModSdkSymbolSeed {
        label: "nte::event::function()",
        insert: "nte::event::function()",
    },
    ModSdkSymbolSeed {
        label: "nte::event::params_size()",
        insert: "nte::event::params_size()",
    },
    ModSdkSymbolSeed {
        label: "nte::event::captured_u64()",
        insert: "nte::event::captured_u64()",
    },
    ModSdkSymbolSeed {
        label: "nte::event::read_u8(offset)",
        insert: "nte::event::read_u8(",
    },
    ModSdkSymbolSeed {
        label: "nte::event::read_u16(offset)",
        insert: "nte::event::read_u16(",
    },
    ModSdkSymbolSeed {
        label: "nte::event::read_u32(offset)",
        insert: "nte::event::read_u32(",
    },
    ModSdkSymbolSeed {
        label: "nte::event::read_u64(offset)",
        insert: "nte::event::read_u64(",
    },
    ModSdkSymbolSeed {
        label: "nte::event::read_i32(offset)",
        insert: "nte::event::read_i32(",
    },
    ModSdkSymbolSeed {
        label: "nte::event::read_f32_milli(offset)",
        insert: "nte::event::read_f32_milli(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::player_character(controller)",
        insert: "nte::sdk::player_character(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::player_state(controller)",
        insert: "nte::sdk::player_state(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::game_paused(controller)",
        insert: "nte::sdk::game_paused(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::attack_target(character)",
        insert: "nte::sdk::attack_target(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::current_weapon(character)",
        insert: "nte::sdk::current_weapon(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::character_level(character)",
        insert: "nte::sdk::character_level(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::character_hp_milli(character)",
        insert: "nte::sdk::character_hp_milli(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::character_hp_max_milli(character, fixed)",
        insert: "nte::sdk::character_hp_max_milli(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::character_is_alive(character)",
        insert: "nte::sdk::character_is_alive(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::character_is_dead(character)",
        insert: "nte::sdk::character_is_dead(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::character_is_controlled(character)",
        insert: "nte::sdk::character_is_controlled(",
    },
    ModSdkSymbolSeed {
        label: "nte::sdk::character_slomo_milli(character)",
        insert: "nte::sdk::character_slomo_milli(",
    },
    ModSdkSymbolSeed {
        label: "nte::cache::get(key)",
        insert: "nte::cache::get(",
    },
    ModSdkSymbolSeed {
        label: "nte::cache::remember(key, value)",
        insert: "nte::cache::remember(",
    },
    ModSdkSymbolSeed {
        label: "nte::equipment::cache_missing()",
        insert: "nte::equipment::cache_missing()",
    },
    ModSdkSymbolSeed {
        label: "nte::equipment::cache_ready(player_state)",
        insert: "nte::equipment::cache_ready(",
    },
    ModSdkSymbolSeed {
        label: "nte::equipment::prepare(player_state)",
        insert: "nte::equipment::prepare(",
    },
    ModSdkSymbolSeed {
        label: "nte::combat_clock::pause_mask(controller)",
        insert: "nte::combat_clock::pause_mask(",
    },
    ModSdkSymbolSeed {
        label: "nte::combat_clock::state_flags(controller)",
        insert: "nte::combat_clock::state_flags(",
    },
    ModSdkSymbolSeed {
        label: "nte::combat_clock::forward(pause_mask, state_flags)",
        insert: "nte::combat_clock::forward(",
    },
    ModSdkSymbolSeed {
        label: "nte::ipc::bind(player_state, controller)",
        insert: "nte::ipc::bind(",
    },
    ModSdkSymbolSeed {
        label: "nte::ipc::emit(\"event\", value...)",
        insert: "nte::ipc::emit(\"event.name\", ",
    },
    ModSdkSymbolSeed {
        label: "nte::ipc::emit(\"pre.event\", value...)",
        insert: "nte::ipc::emit(\"pre.event.name\", ",
    },
    ModSdkSymbolSeed {
        label: "nte::ipc::emit(\"post.event\", value...)",
        insert: "nte::ipc::emit(\"post.event.name\", ",
    },
    ModSdkSymbolSeed {
        label: "nte::time::now_ms()",
        insert: "nte::time::now_ms()",
    },
    ModSdkSymbolSeed {
        label: "nte::log::info(\"message\")",
        insert: "nte::log::info(\"message\")",
    },
    ModSdkSymbolSeed {
        label: "for (std::uint64_t index = 0; index < COUNT; ++index)",
        insert: "for (std::uint64_t index = 0; index < 1; ++index)\n{\n    \n}",
    },
];

fn project_symbol(seed: ModSdkSymbolSeed) -> ModSdkSymbol {
    let kind = symbol_kind(seed.label);
    ModSdkSymbol {
        label: seed.label,
        insert_text: seed.insert,
        kind,
        return_type: symbol_return_type(seed.label, kind),
        documentation_key: symbol_documentation_key(seed.label),
    }
}

fn symbol_kind(label: &str) -> ModSdkSymbolKind {
    if label.starts_with("#include") || label.starts_with("NTE_") || label.starts_with("std::") {
        ModSdkSymbolKind::Declaration
    } else if label.starts_with("void ") || label.starts_with("for ") {
        ModSdkSymbolKind::Snippet
    } else if label.contains('(') {
        ModSdkSymbolKind::Function
    } else {
        ModSdkSymbolKind::Property
    }
}

fn symbol_return_type(label: &str, kind: ModSdkSymbolKind) -> Option<&'static str> {
    if matches!(
        kind,
        ModSdkSymbolKind::Declaration | ModSdkSymbolKind::Snippet
    ) {
        return None;
    }
    let return_type = if label.starts_with("nte::game::") || label == "event.viewport" {
        "std::uintptr_t"
    } else if label.contains("is_readable")
        || label.contains("game_paused")
        || label.contains("character_is_")
        || label.contains("cache_missing")
        || label.contains("cache_ready")
        || label.contains("event::next")
    {
        "bool"
    } else if label.contains("read_u8") {
        "std::uint8_t"
    } else if label.contains("read_u16") || label.contains("params_size") {
        "std::uint16_t"
    } else if label.contains("read_u32") || label.contains("tarray_count") {
        "std::uint32_t"
    } else if label.contains("read_i32") {
        "std::int32_t"
    } else if label.contains("read_ptr")
        || label.contains("tarray_first")
        || label.contains("find_function")
        || label.contains("player_character")
        || label.contains("player_state")
        || label.contains("attack_target")
        || label.contains("current_weapon")
        || label.contains("event::object")
        || label.contains("event::function")
    {
        "std::uintptr_t"
    } else if label.contains("write_")
        || label.contains("params_clear")
        || label.contains("unreal::call")
        || label.contains("unreal::watch")
        || label.contains("unreal::unwatch")
        || label.contains("ipc::bind")
        || label.contains("ipc::emit")
        || label.contains("log::info")
        || label.contains("equipment::prepare")
        || label.contains("combat_clock::forward")
    {
        "bool"
    } else {
        "std::uint64_t"
    };
    Some(return_type)
}

fn symbol_documentation_key(label: &str) -> &'static str {
    if label.starts_with("#include") || label.starts_with("NTE_") {
        "Top-level Mod declaration used by the NTE C++ compiler."
    } else if label.starts_with("void ") || label.starts_with("for ") {
        "Complete code pattern; edit the placeholder values after insertion."
    } else if label.starts_with("nte::game::") || label == "event.viewport" {
        "Stable read-only pointer for the current viewport tick. Requires game.session."
    } else if label.starts_with("nte::memory::") {
        "Checked memory primitive. Add the matching memory.read or memory.write capability."
    } else if label.starts_with("nte::sdk::") {
        "Whitelisted game SDK read. Requires sdk.read."
    } else if label.starts_with("nte::unreal::") {
        "Bounded Unreal reflection or ProcessEvent helper."
    } else if label.starts_with("nte::event::") {
        "Reads a subscribed ProcessEvent record. Requires process.event."
    } else if label.starts_with("nte::cache::") {
        "Per-Mod integer cache retained while the Mod is active."
    } else if label.starts_with("nte::ipc::") {
        "Publishes or binds Mod IPC data. Requires ipc."
    } else if label.starts_with("nte::equipment::") {
        "Equipment host helper. Requires equipment."
    } else if label.starts_with("nte::combat_clock::") {
        "Combat-clock host helper. Requires combat-clock."
    } else if label.starts_with("nte::time::") {
        "Monotonic process time in milliseconds."
    } else if label.starts_with("nte::log::") {
        "Prints a Mod message to the runtime console. Requires log."
    } else {
        "Symbol available to this Mod source file."
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn schema_is_bounded_unique_and_keeps_stable_host_entries() {
        let symbols = mod_sdk_symbols().collect::<Vec<_>>();
        assert_eq!(symbols.len(), 89);
        assert!(symbols.len() <= MAX_MOD_SDK_SYMBOLS);
        assert_eq!(
            symbols
                .iter()
                .map(|symbol| symbol.label)
                .collect::<HashSet<_>>()
                .len(),
            symbols.len()
        );
        assert!(symbols.iter().any(|symbol| {
            symbol.label == "nte::memory::read_ptr(base, offset)"
                && symbol.return_type == Some("std::uintptr_t")
        }));
        assert!(symbols.iter().any(|symbol| {
            symbol.label == "nte::ipc::emit(\"event\", value...)"
                && symbol.kind == ModSdkSymbolKind::Function
        }));
        assert!(symbols.iter().any(|symbol| {
            symbol.label == "NTE_BIND(\"binding-id\");"
                && symbol.kind == ModSdkSymbolKind::Declaration
        }));
    }
}
