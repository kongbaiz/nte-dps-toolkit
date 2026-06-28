use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let resource_dir = manifest_dir.join("res");
    let icon_path = manifest_dir.join("res/icons/app-icon.ico");
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed={}", resource_dir.display());
    println!("cargo:rerun-if-changed={}", icon_path.display());

    generate_embedded_resources(&manifest_dir, &resource_dir, &output_dir);

    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon(icon_path.to_str().expect("icon path must be valid UTF-8"))
        .compile()
        .expect("failed to embed Windows application icon");
}

fn generate_embedded_resources(manifest_dir: &Path, resource_dir: &Path, output_dir: &Path) {
    let mut resources = Vec::new();
    collect_resources(resource_dir, &mut resources);
    resources.sort();

    let mut generated = String::from(concat!(
        "fn embedded_resource(path: &str) -> Option<&'static [u8]> {\n",
        "  let normalized = path.replace('\\\\', \"/\");\n",
        "  match normalized.as_str() {\n",
    ));

    for resource in resources {
        println!("cargo:rerun-if-changed={}", resource.display());
        let relative = resource
            .strip_prefix(manifest_dir)
            .expect("resource must be inside the project")
            .to_string_lossy()
            .replace('\\', "/");
        let absolute = resource.to_string_lossy();
        generated.push_str(&format!(
            "        {relative:?} => Some(include_bytes!({absolute:?})),\n"
        ));
    }

    generated.push_str("        _ => None,\n    }\n}\n");
    let output_path = output_dir.join("embedded_resources.rs");
    fs::write(output_path, generated).expect("failed to generate embedded resource map");
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
