use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde_json::Value;

const EXCLUDED_EMBEDDED_RESOURCES: &[&str] = &[
    "res/data/asset_manifest.json",
    "res/data/asset_report.json",
    "res/data/core_manifest.json",
    "res/icons/app-icon.ico",
];

const CORE_MANIFEST_PATH: &str = "res/data/core_manifest.json";
const EMBEDDED_ABYSS_DISPLAY_NAMES: &str = "res/data/abyss/monster_stat_names_zh_cn.json";

#[derive(Clone, Copy)]
enum ResourceMode {
    External,
    Full,
    Core,
}

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let resource_dir = manifest_dir.join("res");
    let icon_path = manifest_dir.join("res/icons/app-icon.ico");
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let desktop_enabled = env::var_os("CARGO_FEATURE_DESKTOP").is_some();
    let cli_enabled = env::var_os("CARGO_FEATURE_CLI").is_some();
    let tauri_frontend_enabled = env::var_os("CARGO_FEATURE_TAURI_FRONTEND").is_some();
    let mode = if env::var_os("CARGO_FEATURE_EXTERNAL_RESOURCES").is_some() {
        ResourceMode::External
    } else if cli_enabled && !desktop_enabled {
        ResourceMode::Core
    } else {
        ResourceMode::Full
    };

    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_EXTERNAL_RESOURCES");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_DESKTOP");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_CLI");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TAURI_FRONTEND");
    println!("cargo:rerun-if-env-changed=NTE_EMBEDDED_RESOURCE_REPORT");
    match mode {
        ResourceMode::Full => println!("cargo:rerun-if-changed={}", resource_dir.display()),
        ResourceMode::Core => println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join(CORE_MANIFEST_PATH).display()
        ),
        ResourceMode::External => {}
    }
    if desktop_enabled {
        println!("cargo:rerun-if-changed={}", icon_path.display());
    }

    generate_embedded_resources(
        &manifest_dir,
        &resource_dir,
        &output_dir,
        mode,
        tauri_frontend_enabled,
    );

    #[cfg(windows)]
    if desktop_enabled {
        winresource::WindowsResource::new()
            .set_icon(icon_path.to_str().expect("icon path must be valid UTF-8"))
            .compile()
            .expect("failed to embed Windows application icon");
    }
}

fn generate_embedded_resources(
    manifest_dir: &Path,
    resource_dir: &Path,
    output_dir: &Path,
    mode: ResourceMode,
    tauri_frontend_enabled: bool,
) {
    if matches!(mode, ResourceMode::External) {
        let generated = concat!(
            "#[allow(dead_code)]\n",
            "#[derive(Clone, Copy)]\n",
            "enum EmbeddedResourceEncoding { Original, ZlibJson }\n",
            "#[allow(dead_code)]\n",
            "#[derive(Clone, Copy)]\n",
            "struct EmbeddedResourceEntry {\n",
            "    bytes: &'static [u8],\n",
            "    decoded_len: usize,\n",
            "    encoding: EmbeddedResourceEncoding,\n",
            "}\n",
            "fn embedded_resource(_path: &str) -> Option<EmbeddedResourceEntry> {\n",
            "    None\n",
            "}\n",
            "fn frontend_resource_exists(_path: &str) -> bool {\n",
            "    false\n",
            "}\n",
        );
        let output_path = output_dir.join("embedded_resources.rs");
        fs::write(output_path, generated).expect("failed to generate embedded resource map");
        return;
    }

    let mut resources = Vec::new();
    match mode {
        ResourceMode::Full => collect_resources(resource_dir, &mut resources),
        ResourceMode::Core => resources = core_resources(manifest_dir),
        ResourceMode::External => unreachable!("external resources returned above"),
    }
    resources.sort();
    let transformed_dir = output_dir.join("embedded_resources");
    if transformed_dir.exists() {
        fs::remove_dir_all(&transformed_dir)
            .expect("failed to clean generated embedded resource directory");
    }
    fs::create_dir_all(&transformed_dir)
        .expect("failed to create generated embedded resource directory");

    let mut generated = String::from(concat!(
        "#[allow(dead_code)]\n",
        "#[derive(Clone, Copy)]\n",
        "enum EmbeddedResourceEncoding { Original, ZlibJson }\n",
        "#[allow(dead_code)]\n",
        "#[derive(Clone, Copy)]\n",
        "struct EmbeddedResourceEntry {\n",
        "    bytes: &'static [u8],\n",
        "    decoded_len: usize,\n",
        "    encoding: EmbeddedResourceEncoding,\n",
        "}\n",
        "fn embedded_resource(path: &str) -> Option<EmbeddedResourceEntry> {\n",
        "  let normalized = path.replace('\\\\', \"/\");\n",
        "  match normalized.as_str() {\n",
    ));
    let mut frontend_resources = String::from(concat!(
        "#[allow(clippy::match_like_matches_macro)]\n",
        "#[allow(clippy::match_single_binding)]\n",
        "fn frontend_resource_exists(path: &str) -> bool {\n",
        "  let normalized = path.replace('\\\\', \"/\");\n",
        "  match normalized.as_str() {\n",
    ));

    let mut original_bytes = 0_u64;
    let mut embedded_bytes = 0_u64;
    let mut skipped = 0_usize;
    let mut compressed_json = 0_usize;
    let mut json_source_bytes = 0_u64;
    let mut json_decoded_bytes = 0_u64;
    let mut json_compressed_bytes = 0_u64;
    let mut frontend_images = 0_usize;
    let mut frontend_image_bytes = 0_u64;

    for resource in resources {
        println!("cargo:rerun-if-changed={}", resource.display());
        let relative = resource
            .strip_prefix(manifest_dir)
            .expect("resource must be inside the project")
            .to_string_lossy()
            .replace('\\', "/");
        if should_exclude_embedded_resource(&relative) {
            skipped += 1;
            continue;
        }
        let original = fs::read(&resource).expect("failed to read resource for embedding");
        original_bytes += original.len() as u64;
        if tauri_frontend_enabled && is_frontend_image_resource(&relative) {
            frontend_images += 1;
            frontend_image_bytes += original.len() as u64;
            frontend_resources.push_str(&format!("        {relative:?} => true,\n"));
            continue;
        }
        let processed = process_embedded_resource(&relative, &original);
        match processed.kind {
            EmbeddedResourceKind::ZlibJson => {
                compressed_json += 1;
                json_source_bytes += original.len() as u64;
                json_decoded_bytes += processed.decoded_len as u64;
                json_compressed_bytes += processed.bytes.len() as u64;
            }
            EmbeddedResourceKind::Original => {}
        }
        embedded_bytes += processed.bytes.len() as u64;
        let generated_path =
            transformed_dir.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = generated_path.parent() {
            fs::create_dir_all(parent).expect("failed to create generated resource parent");
        }
        fs::write(&generated_path, &processed.bytes)
            .expect("failed to write generated embedded resource");
        let absolute = generated_path.to_string_lossy();
        let encoding = match processed.kind {
            EmbeddedResourceKind::ZlibJson => "EmbeddedResourceEncoding::ZlibJson",
            EmbeddedResourceKind::Original => "EmbeddedResourceEncoding::Original",
        };
        generated.push_str(&format!(
            "        {relative:?} => Some(EmbeddedResourceEntry {{ bytes: include_bytes!({absolute:?}), decoded_len: {}, encoding: {encoding} }}),\n",
            processed.decoded_len
        ));
    }

    generated.push_str("        _ => None,\n    }\n}\n");
    frontend_resources.push_str("        _ => false,\n    }\n}\n");
    generated.push_str(&frontend_resources);
    let output_path = output_dir.join("embedded_resources.rs");
    fs::write(output_path, generated).expect("failed to generate embedded resource map");
    if env::var_os("NTE_EMBEDDED_RESOURCE_REPORT").is_some() {
        println!(
            "cargo:warning=embedded resources: skipped {skipped}, frontend_images {frontend_images} ({frontend_image_bytes} bytes), compressed_json {compressed_json}, bytes {} -> {}",
            original_bytes, embedded_bytes
        );
        println!(
            "cargo:warning=embedded JSON: source {json_source_bytes}, minified {json_decoded_bytes}, zlib {json_compressed_bytes} bytes"
        );
    }
}

fn core_resources(manifest_dir: &Path) -> Vec<PathBuf> {
    let manifest_path = manifest_dir.join(CORE_MANIFEST_PATH);
    let bytes = fs::read(&manifest_path).expect("failed to read core resource manifest");
    let document: Value =
        serde_json::from_slice(&bytes).expect("core resource manifest must be valid JSON");
    let object = document
        .as_object()
        .expect("core resource manifest must be a JSON object");
    assert_eq!(
        object.get("format_version").and_then(Value::as_u64),
        Some(1),
        "unsupported core resource manifest format"
    );
    assert!(
        object
            .get("data_version")
            .and_then(Value::as_str)
            .is_some_and(|version| !version.is_empty()),
        "core resource manifest requires data_version"
    );
    let resources = object
        .get("resources")
        .and_then(Value::as_array)
        .expect("core resource manifest requires a resources array");
    let mut relative_paths = resources
        .iter()
        .map(|resource| {
            let relative = resource
                .as_str()
                .expect("core resource paths must be strings");
            assert!(
                relative.starts_with("res/data/")
                    && relative.ends_with(".json")
                    && !relative.contains('\\')
                    && !relative.split('/').any(|part| part == "." || part == ".."),
                "invalid core resource path: {relative}"
            );
            relative.to_owned()
        })
        .collect::<Vec<_>>();
    relative_paths.sort();
    for duplicate in relative_paths.windows(2) {
        assert_ne!(duplicate[0], duplicate[1], "duplicate core resource path");
    }
    relative_paths
        .into_iter()
        .map(|relative| {
            let path = manifest_dir.join(&relative);
            assert!(path.is_file(), "core resource does not exist: {relative}");
            println!("cargo:rerun-if-changed={}", path.display());
            path
        })
        .collect()
}

fn collect_resources(directory: &Path, resources: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries {
        let entry = entry.expect("failed to read res directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_resources(&path, resources);
            continue;
        }

        resources.push(path);
    }
}

fn should_exclude_embedded_resource(relative: &str) -> bool {
    (relative.starts_with("res/data/abyss/") && relative != EMBEDDED_ABYSS_DISPLAY_NAMES)
        || EXCLUDED_EMBEDDED_RESOURCES
            .iter()
            .any(|excluded| relative.eq_ignore_ascii_case(excluded))
}

fn is_frontend_image_resource(relative: &str) -> bool {
    relative.starts_with("res/images/") && relative.ends_with(".png")
}

struct EmbeddedResource {
    bytes: Vec<u8>,
    decoded_len: usize,
    kind: EmbeddedResourceKind,
}

enum EmbeddedResourceKind {
    Original,
    ZlibJson,
}

fn process_embedded_resource(relative: &str, original: &[u8]) -> EmbeddedResource {
    if relative.ends_with(".json") {
        let document = serde_json::from_slice::<Value>(original)
            .unwrap_or_else(|error| panic!("invalid JSON resource {relative}: {error}"));
        let minified = serde_json::to_vec(&document).expect("failed to minify JSON resource");
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder
            .write_all(&minified)
            .expect("failed to compress JSON resource");
        let bytes = encoder.finish().expect("failed to finish JSON compression");
        return EmbeddedResource {
            bytes,
            decoded_len: minified.len(),
            kind: EmbeddedResourceKind::ZlibJson,
        };
    }

    EmbeddedResource {
        bytes: original.to_vec(),
        decoded_len: original.len(),
        kind: EmbeddedResourceKind::Original,
    }
}
