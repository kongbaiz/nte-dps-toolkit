use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const IMAGE_EXTENSIONS: &[&str] = &["jpeg", "jpg", "png", "webp"];

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let resource_dir = manifest_dir.join("res");
    let icon_path = manifest_dir.join("res/icons/app-icon.ico");
    let abyss_data_path = manifest_dir.join("data/DataTable/PackData/DT_MonsterPackData.json");
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed={}", resource_dir.display());
    println!("cargo:rerun-if-changed={}", icon_path.display());
    println!("cargo:rerun-if-changed={}", abyss_data_path.display());

    generate_embedded_resources(&manifest_dir, &resource_dir, &output_dir);
    generate_abyss_stage_index(&abyss_data_path, &output_dir);

    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon(icon_path.to_str().expect("icon path must be valid UTF-8"))
        .compile()
        .expect("failed to embed Windows application icon");
}

fn generate_embedded_resources(manifest_dir: &Path, resource_dir: &Path, output_dir: &Path) {
    let mut images = Vec::new();
    collect_images(resource_dir, &mut images);
    images.sort();

    let mut generated = String::from(
        "fn embedded_image_resource(path: &str) -> Option<&'static [u8]> {\n\
         \x20   let normalized = path.replace('\\\\', \"/\");\n\
         \x20   match normalized.as_str() {\n",
    );

    for image in images {
        println!("cargo:rerun-if-changed={}", image.display());
        let relative = image
            .strip_prefix(manifest_dir)
            .expect("resource must be inside the project")
            .to_string_lossy()
            .replace('\\', "/");
        let absolute = image.to_string_lossy();
        generated.push_str(&format!(
            "        {relative:?} => Some(include_bytes!({absolute:?})),\n"
        ));
    }

    generated.push_str("        _ => None,\n    }\n}\n");
    let output_path = output_dir.join("embedded_resources.rs");
    fs::write(output_path, generated).expect("failed to generate embedded resource map");
}

fn generate_abyss_stage_index(data_path: &Path, output_dir: &Path) {
    let contents = fs::read(data_path).expect("failed to read Abyss monster pack data");
    let document: Value =
        serde_json::from_slice(&contents).expect("invalid Abyss monster pack data");
    let rows = document
        .as_array()
        .and_then(|tables| tables.first())
        .and_then(|table| table.get("Rows"))
        .and_then(Value::as_object)
        .expect("Abyss monster pack data must contain Rows");

    let mut stages = Vec::new();
    for (key, row) in rows {
        let mut parts = key.split('_');
        if parts.next() != Some("Abyss") {
            continue;
        }
        let (Some(cycle), Some(floor), Some(half), Some(wave)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(cycle), Ok(floor), Ok(half), Ok(wave)) = (
            cycle.parse::<u32>(),
            floor.parse::<u32>(),
            half.parse::<u8>(),
            wave.parse::<u32>(),
        ) else {
            continue;
        };
        if half > 1 {
            continue;
        }
        let monster = parts.collect::<Vec<_>>().join("_");
        if monster.is_empty() {
            continue;
        }
        let Some(max_hp) = row
            .get("HPMaxBase")
            .and_then(Value::as_f64)
            .map(|value| value.round() as u64)
            .filter(|value| *value > 0)
        else {
            continue;
        };
        stages.push((cycle, floor, half, wave, monster, max_hp));
    }
    stages.sort();

    let mut generated =
        String::from("const ABYSS_STAGE_ROWS: &[(u32, u32, u8, u32, &str, u64)] = &[\n");
    for (cycle, floor, half, wave, monster, max_hp) in stages {
        generated.push_str(&format!(
            "    ({cycle}, {floor}, {half}, {wave}, {monster:?}, {max_hp}),\n"
        ));
    }
    generated.push_str("];\n");
    fs::write(output_dir.join("abyss_stage_index.rs"), generated)
        .expect("failed to generate Abyss stage index");
}

fn collect_images(directory: &Path, images: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries {
        let entry = entry.expect("failed to read res directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_images(&path, images);
            continue;
        }

        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if extension
            .as_deref()
            .is_some_and(|extension| IMAGE_EXTENSIONS.contains(&extension))
        {
            images.push(path);
        }
    }
}
