use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use image::ImageEncoder;
use image::codecs::webp::WebPEncoder;
use serde_json::Value;

const EXCLUDED_EMBEDDED_RESOURCES: &[&str] = &[
    "res/data/asset_manifest.json",
    "res/data/asset_report.json",
    "res/data/core_manifest.json",
    "res/icons/app-icon.ico",
];

const CORE_MANIFEST_PATH: &str = "res/data/core_manifest.json";

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
    let gui_enabled = env::var_os("CARGO_FEATURE_GUI").is_some();
    let cli_enabled = env::var_os("CARGO_FEATURE_CLI").is_some();
    let mode = if env::var_os("CARGO_FEATURE_EXTERNAL_RESOURCES").is_some() {
        ResourceMode::External
    } else if cli_enabled && !gui_enabled {
        ResourceMode::Core
    } else {
        ResourceMode::Full
    };

    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_EXTERNAL_RESOURCES");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_GUI");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_CLI");
    match mode {
        ResourceMode::Full => println!("cargo:rerun-if-changed={}", resource_dir.display()),
        ResourceMode::Core => println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join(CORE_MANIFEST_PATH).display()
        ),
        ResourceMode::External => {}
    }
    if gui_enabled {
        println!("cargo:rerun-if-changed={}", icon_path.display());
    }

    generate_embedded_resources(&manifest_dir, &resource_dir, &output_dir, mode);

    #[cfg(windows)]
    if gui_enabled {
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
) {
    if matches!(mode, ResourceMode::External) {
        let generated = concat!(
            "fn embedded_resource(_path: &str) -> Option<&'static [u8]> {\n",
            "    None\n",
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
        "fn embedded_resource(path: &str) -> Option<&'static [u8]> {\n",
        "  let normalized = path.replace('\\\\', \"/\");\n",
        "  match normalized.as_str() {\n",
    ));

    let mut original_bytes = 0_u64;
    let mut embedded_bytes = 0_u64;
    let mut skipped = 0_usize;
    let mut minified_json = 0_usize;
    let mut webp_images = 0_usize;

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
        let processed = process_embedded_resource(&relative, &original);
        match processed.kind {
            EmbeddedResourceKind::Json => minified_json += 1,
            EmbeddedResourceKind::Webp => webp_images += 1,
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
        generated.push_str(&format!(
            "        {relative:?} => Some(include_bytes!({absolute:?})),\n"
        ));
    }

    generated.push_str("        _ => None,\n    }\n}\n");
    let output_path = output_dir.join("embedded_resources.rs");
    fs::write(output_path, generated).expect("failed to generate embedded resource map");
    if env::var_os("NTE_EMBEDDED_RESOURCE_REPORT").is_some() {
        println!(
            "cargo:warning=embedded resources: skipped {skipped}, minified_json {minified_json}, webp_images {webp_images}, bytes {} -> {}",
            original_bytes, embedded_bytes
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
    EXCLUDED_EMBEDDED_RESOURCES
        .iter()
        .any(|excluded| relative.eq_ignore_ascii_case(excluded))
}

struct EmbeddedResource {
    bytes: Vec<u8>,
    kind: EmbeddedResourceKind,
}

enum EmbeddedResourceKind {
    Original,
    Json,
    Webp,
}

fn process_embedded_resource(relative: &str, original: &[u8]) -> EmbeddedResource {
    if relative.ends_with(".json")
        && let Ok(document) = serde_json::from_slice::<Value>(original)
    {
        return EmbeddedResource {
            bytes: serde_json::to_vec(&document).expect("failed to minify JSON resource"),
            kind: EmbeddedResourceKind::Json,
        };
    }

    if relative.ends_with(".png")
        && let Some(webp) = png_to_lossless_webp(original)
        && webp.len() < original.len()
    {
        return EmbeddedResource {
            bytes: webp,
            kind: EmbeddedResourceKind::Webp,
        };
    }

    EmbeddedResource {
        bytes: original.to_vec(),
        kind: EmbeddedResourceKind::Original,
    }
}

fn png_to_lossless_webp(original: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(original).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    let mut bytes = Vec::new();
    WebPEncoder::new_lossless(&mut bytes)
        .write_image(
            image.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(bytes)
}
