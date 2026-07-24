use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signature, VerifyingKey};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

pub const UPDATE_SCHEMA_VERSION: u32 = 1;
pub const UPDATER_PROTOCOL_VERSION: u32 = 1;
pub const PRODUCT_ID: &str = "nte-dps-tool";
pub const UPDATE_CHANNEL: &str = "stable";
pub const UPDATE_PLATFORM: &str = "windows-x86_64";
#[cfg(not(feature = "external_resources"))]
pub const UPDATE_VARIANT: &str = "standard";
#[cfg(feature = "external_resources")]
pub const UPDATE_VARIANT: &str = "external-resources";
pub const APP_COMPONENT_ID: &str = "app";
pub const EQUIPMENT_PLUGIN_COMPONENT_ID: &str = "equipment-plugin";
pub const UPDATE_HEALTH_MARKER_ENV: &str = "NTE_UPDATE_HEALTH_MARKER";
pub const MAX_MANIFEST_BYTES: usize = 128 * 1024;
pub const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
#[cfg(not(feature = "external_resources"))]
pub const DEFAULT_MANIFEST_URL: &str =
    "https://dps.o-na-ni.com/updates/v1/stable/windows-x86_64/standard/manifest.json";
#[cfg(feature = "external_resources")]
pub const DEFAULT_MANIFEST_URL: &str =
    "https://dps.o-na-ni.com/updates/v1/stable/windows-x86_64/external-resources/manifest.json";

const COMPILED_UPDATE_KEY_ID: Option<&str> = option_env!("NTE_UPDATE_KEY_ID");
const COMPILED_UPDATE_PUBLIC_KEY_HEX: Option<&str> = option_env!("NTE_UPDATE_PUBLIC_KEY_HEX");
const COMPILED_UPDATE_MANIFEST_URL: Option<&str> = option_env!("NTE_UPDATE_MANIFEST_URL");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateEndpoint {
    pub manifest_url: String,
    pub key_id: String,
    pub public_key: [u8; 32],
}

impl UpdateEndpoint {
    pub fn official() -> Result<Self, UpdateError> {
        let key_id = COMPILED_UPDATE_KEY_ID
            .filter(|value| !value.trim().is_empty())
            .ok_or(UpdateError::ClientNotConfigured)?;
        let public_key_hex = COMPILED_UPDATE_PUBLIC_KEY_HEX
            .filter(|value| !value.trim().is_empty())
            .ok_or(UpdateError::ClientNotConfigured)?;
        let public_key = decode_fixed_hex::<32>(public_key_hex)
            .map_err(|_| UpdateError::InvalidCompiledPublicKey)?;
        let manifest_url = COMPILED_UPDATE_MANIFEST_URL
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(DEFAULT_MANIFEST_URL)
            .to_owned();
        validate_https_url(&manifest_url)?;
        Ok(Self {
            manifest_url,
            key_id: key_id.to_owned(),
            public_key,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
struct SignedManifestEnvelope {
    payload: String,
    signatures: Vec<ManifestSignature>,
}

#[derive(Clone, Debug, Deserialize)]
struct ManifestSignature {
    key_id: String,
    signature: String,
}

#[derive(Clone, Debug, Deserialize)]
struct UpdateManifest {
    schema: u32,
    product: String,
    channel: String,
    release_id: String,
    published_at: String,
    #[serde(default)]
    notes: String,
    updater_protocol: u32,
    components: Vec<ComponentRelease>,
}

#[derive(Clone, Debug, Deserialize)]
struct ComponentRelease {
    id: String,
    version: String,
    platform: String,
    variant: String,
    #[serde(default)]
    requires_app: Option<String>,
    artifact: UpdateArtifact,
}

#[derive(Clone, Debug, Deserialize)]
struct UpdateArtifact {
    url: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateComponent {
    App,
    EquipmentPlugin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledComponentVersions {
    pub app: Version,
    pub equipment_plugin: Option<Version>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableComponentUpdate {
    pub component: UpdateComponent,
    pub release_id: String,
    pub version: Version,
    pub published_at: String,
    pub notes: String,
    pub artifact_url: String,
    pub artifact_size: u64,
    pub artifact_sha256: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateTransaction {
    pub schema: u32,
    pub id: String,
    pub parent_pid: u32,
    pub install_dir: PathBuf,
    pub staging_dir: PathBuf,
    pub health_marker: PathBuf,
    pub files: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    ClientNotConfigured,
    InvalidCompiledPublicKey,
    ManifestTooLarge,
    InvalidEnvelope,
    MissingTrustedSignature,
    InvalidSignature,
    InvalidPayload,
    UnsupportedSchema(u32),
    WrongProduct,
    WrongChannel,
    InvalidReleaseId,
    UnsupportedUpdaterProtocol(u32),
    InvalidVersion,
    InvalidVersionRequirement,
    DuplicateComponent(String),
    InvalidArtifactUrl,
    InvalidArtifactSize,
    InvalidArtifactHash,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClientNotConfigured => {
                formatter.write_str("official update key is not configured in this build")
            }
            Self::InvalidCompiledPublicKey => {
                formatter.write_str("compiled update public key is invalid")
            }
            Self::ManifestTooLarge => formatter.write_str("update manifest exceeds the size limit"),
            Self::InvalidEnvelope => formatter.write_str("update manifest envelope is invalid"),
            Self::MissingTrustedSignature => {
                formatter.write_str("update manifest has no signature from the trusted key")
            }
            Self::InvalidSignature => formatter.write_str("update manifest signature is invalid"),
            Self::InvalidPayload => formatter.write_str("signed update payload is invalid"),
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "unsupported update manifest schema {schema}")
            }
            Self::WrongProduct => formatter.write_str("update manifest targets another product"),
            Self::WrongChannel => formatter.write_str("update manifest targets another channel"),
            Self::InvalidReleaseId => formatter.write_str("update release ID is invalid"),
            Self::UnsupportedUpdaterProtocol(version) => {
                write!(formatter, "unsupported updater protocol {version}")
            }
            Self::InvalidVersion => formatter.write_str("update version is invalid"),
            Self::InvalidVersionRequirement => {
                formatter.write_str("component compatibility requirement is invalid")
            }
            Self::DuplicateComponent(component) => {
                write!(
                    formatter,
                    "update manifest contains duplicate {component} components"
                )
            }
            Self::InvalidArtifactUrl => formatter.write_str("update artifact URL is invalid"),
            Self::InvalidArtifactSize => formatter.write_str("update artifact size is invalid"),
            Self::InvalidArtifactHash => formatter.write_str("update artifact hash is invalid"),
        }
    }
}

impl std::error::Error for UpdateError {}

pub fn verify_manifest(
    bytes: &[u8],
    endpoint: &UpdateEndpoint,
    installed: &InstalledComponentVersions,
) -> Result<Vec<AvailableComponentUpdate>, UpdateError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(UpdateError::ManifestTooLarge);
    }
    let envelope: SignedManifestEnvelope =
        serde_json::from_slice(bytes).map_err(|_| UpdateError::InvalidEnvelope)?;
    let payload = BASE64
        .decode(envelope.payload.as_bytes())
        .map_err(|_| UpdateError::InvalidEnvelope)?;
    if payload.len() > MAX_MANIFEST_BYTES {
        return Err(UpdateError::ManifestTooLarge);
    }
    let trusted_signature = envelope
        .signatures
        .iter()
        .find(|signature| signature.key_id == endpoint.key_id)
        .ok_or(UpdateError::MissingTrustedSignature)?;
    let signature_bytes = BASE64
        .decode(trusted_signature.signature.as_bytes())
        .map_err(|_| UpdateError::InvalidSignature)?;
    let signature =
        Signature::from_slice(&signature_bytes).map_err(|_| UpdateError::InvalidSignature)?;
    let key = VerifyingKey::from_bytes(&endpoint.public_key)
        .map_err(|_| UpdateError::InvalidCompiledPublicKey)?;
    key.verify_strict(&payload, &signature)
        .map_err(|_| UpdateError::InvalidSignature)?;

    let manifest: UpdateManifest =
        serde_json::from_slice(&payload).map_err(|_| UpdateError::InvalidPayload)?;
    validate_manifest(manifest, installed)
}

fn validate_manifest(
    manifest: UpdateManifest,
    installed: &InstalledComponentVersions,
) -> Result<Vec<AvailableComponentUpdate>, UpdateError> {
    if manifest.schema != UPDATE_SCHEMA_VERSION {
        return Err(UpdateError::UnsupportedSchema(manifest.schema));
    }
    if manifest.product != PRODUCT_ID {
        return Err(UpdateError::WrongProduct);
    }
    if manifest.channel != UPDATE_CHANNEL {
        return Err(UpdateError::WrongChannel);
    }
    if !valid_release_id(&manifest.release_id) {
        return Err(UpdateError::InvalidReleaseId);
    }
    if manifest.updater_protocol > UPDATER_PROTOCOL_VERSION {
        return Err(UpdateError::UnsupportedUpdaterProtocol(
            manifest.updater_protocol,
        ));
    }
    let mut updates = Vec::new();
    let mut seen_app = false;
    let mut seen_equipment_plugin = false;
    for component in manifest.components {
        if component.platform != UPDATE_PLATFORM || component.variant != UPDATE_VARIANT {
            continue;
        }
        let (kind, seen) = match component.id.as_str() {
            APP_COMPONENT_ID => (UpdateComponent::App, &mut seen_app),
            EQUIPMENT_PLUGIN_COMPONENT_ID => {
                (UpdateComponent::EquipmentPlugin, &mut seen_equipment_plugin)
            }
            _ => continue,
        };
        if *seen {
            return Err(UpdateError::DuplicateComponent(component.id));
        }
        *seen = true;

        let version =
            Version::parse(&component.version).map_err(|_| UpdateError::InvalidVersion)?;
        let requirement = component
            .requires_app
            .map(|requirement| {
                VersionReq::parse(&requirement).map_err(|_| UpdateError::InvalidVersionRequirement)
            })
            .transpose()?;
        validate_https_url(&component.artifact.url)?;
        if component.artifact.size == 0 || component.artifact.size > MAX_PACKAGE_BYTES {
            return Err(UpdateError::InvalidArtifactSize);
        }
        let artifact_sha256 = decode_fixed_hex::<32>(&component.artifact.sha256)
            .map_err(|_| UpdateError::InvalidArtifactHash)?;

        if requirement.is_some_and(|requirement| !requirement.matches(&installed.app)) {
            continue;
        }
        let current = match kind {
            UpdateComponent::App => Some(&installed.app),
            UpdateComponent::EquipmentPlugin => installed.equipment_plugin.as_ref(),
        };
        if current.is_some_and(|current| version <= *current) {
            continue;
        }
        updates.push(AvailableComponentUpdate {
            component: kind,
            release_id: manifest.release_id.clone(),
            version,
            published_at: manifest.published_at.clone(),
            notes: manifest.notes.clone(),
            artifact_url: component.artifact.url,
            artifact_size: component.artifact.size,
            artifact_sha256,
        });
    }
    updates.sort_by_key(|update| match update.component {
        UpdateComponent::App => 0,
        UpdateComponent::EquipmentPlugin => 1,
    });
    Ok(updates)
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], ()> {
    let bytes = hex::decode(value.trim()).map_err(|_| ())?;
    bytes.try_into().map_err(|_| ())
}

fn validate_https_url(url: &str) -> Result<(), UpdateError> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(UpdateError::InvalidArtifactUrl);
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty()
        || authority.contains('@')
        || authority.chars().any(char::is_whitespace)
        || url.contains(['\r', '\n'])
    {
        return Err(UpdateError::InvalidArtifactUrl);
    }
    Ok(())
}

fn valid_release_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub fn safe_managed_release_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    for component in path.components() {
        let Component::Normal(name) = component else {
            return false;
        };
        if !safe_windows_release_name(name) {
            return false;
        }
    }
    matches!(
        path.to_string_lossy().replace('\\', "/").as_str(),
        "nte-dps-tool.exe"
            | "nte-updater.exe"
            | "BUILD_VARIANT.md"
            | "THIRD_PARTY_LICENSES.md"
            | "NOTICE.md"
    ) || path.starts_with("plugins")
        || path.starts_with("licenses")
        || path.starts_with("res")
}

fn safe_windows_release_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if name.is_empty()
        || name.encode_utf16().count() > 255
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|character| character.is_control() || r#"<>:"/\|?*"#.contains(character))
    {
        return false;
    }
    let base = name.split('.').next().expect("file name is not empty");
    let base = base.to_ascii_uppercase();
    let reserved = matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    );
    !reserved
        && !(base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use serde_json::json;

    fn signed_manifest(payload: serde_json::Value) -> (Vec<u8>, UpdateEndpoint) {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let payload = serde_json::to_vec(&payload).unwrap();
        let signature = signing_key.sign(&payload);
        let envelope = json!({
            "payload": BASE64.encode(&payload),
            "signatures": [{
                "key_id": "test-key",
                "signature": BASE64.encode(signature.to_bytes()),
            }],
        });
        (
            serde_json::to_vec(&envelope).unwrap(),
            UpdateEndpoint {
                manifest_url: "https://updates.example.test/manifest.json".to_owned(),
                key_id: "test-key".to_owned(),
                public_key: signing_key.verifying_key().to_bytes(),
            },
        )
    }

    fn payload(version: &str) -> serde_json::Value {
        json!({
            "schema": 1,
            "product": "nte-dps-tool",
            "channel": "stable",
            "release_id": "release-0.3.5",
            "published_at": "2026-07-24T12:00:00Z",
            "notes": "Updater client",
            "updater_protocol": 1,
            "components": [{
                "id": "app",
                "version": version,
                "platform": "windows-x86_64",
                "variant": UPDATE_VARIANT,
                "requires_app": ">=0.3.0,<0.4.0",
                "artifact": {
                    "url": "https://updates.example.test/releases/0.3.5/app.zip",
                    "size": 1024,
                    "sha256": "11".repeat(32),
                }
            }]
        })
    }

    #[test]
    fn accepts_newer_signed_app_release() {
        let (bytes, endpoint) = signed_manifest(payload("0.3.6"));
        let updates = verify_manifest(
            &bytes,
            &endpoint,
            &InstalledComponentVersions {
                app: Version::parse("0.3.5").unwrap(),
                equipment_plugin: Some(Version::parse("0.3.5").unwrap()),
            },
        )
        .unwrap();

        assert_eq!(updates[0].component, UpdateComponent::App);
        assert_eq!(updates[0].version, Version::parse("0.3.6").unwrap());
    }

    #[test]
    fn ignores_current_or_older_release() {
        let (bytes, endpoint) = signed_manifest(payload("0.3.5"));
        let updates = verify_manifest(
            &bytes,
            &endpoint,
            &InstalledComponentVersions {
                app: Version::parse("0.3.5").unwrap(),
                equipment_plugin: Some(Version::parse("0.3.5").unwrap()),
            },
        )
        .unwrap();

        assert!(updates.is_empty());
    }

    #[test]
    fn accepts_plugin_release_without_newer_app() {
        let mut document = payload("0.3.5");
        document["components"]
            .as_array_mut()
            .expect("components should be an array")
            .push(json!({
                "id": EQUIPMENT_PLUGIN_COMPONENT_ID,
                "version": "0.3.6",
                "platform": "windows-x86_64",
                "variant": UPDATE_VARIANT,
                "requires_app": ">=0.3.5,<0.4.0",
                "artifact": {
                    "url": "https://updates.example.test/releases/0.3.6/equipment-plugin.zip",
                    "size": 512,
                    "sha256": "22".repeat(32),
                }
            }));
        let (bytes, endpoint) = signed_manifest(document);

        let updates = verify_manifest(
            &bytes,
            &endpoint,
            &InstalledComponentVersions {
                app: Version::parse("0.3.5").unwrap(),
                equipment_plugin: Some(Version::parse("0.3.5").unwrap()),
            },
        )
        .unwrap();

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].component, UpdateComponent::EquipmentPlugin);
        assert_eq!(updates[0].version, Version::parse("0.3.6").unwrap());
    }

    #[test]
    fn ignores_plugin_incompatible_with_installed_app() {
        let mut document = payload("0.3.5");
        document["components"]
            .as_array_mut()
            .expect("components should be an array")
            .push(json!({
                "id": EQUIPMENT_PLUGIN_COMPONENT_ID,
                "version": "0.3.6",
                "platform": "windows-x86_64",
                "variant": UPDATE_VARIANT,
                "requires_app": ">=0.4.0",
                "artifact": {
                    "url": "https://updates.example.test/releases/0.3.6/equipment-plugin.zip",
                    "size": 512,
                    "sha256": "22".repeat(32),
                }
            }));
        let (bytes, endpoint) = signed_manifest(document);

        let updates = verify_manifest(
            &bytes,
            &endpoint,
            &InstalledComponentVersions {
                app: Version::parse("0.3.5").unwrap(),
                equipment_plugin: Some(Version::parse("0.3.5").unwrap()),
            },
        )
        .unwrap();

        assert!(updates.is_empty());
    }

    #[test]
    fn rejects_payload_changed_after_signing() {
        let (bytes, endpoint) = signed_manifest(payload("0.3.6"));
        let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        envelope["payload"] = serde_json::Value::String(BASE64.encode(b"{}"));

        let error = verify_manifest(
            &serde_json::to_vec(&envelope).unwrap(),
            &endpoint,
            &InstalledComponentVersions {
                app: Version::parse("0.3.5").unwrap(),
                equipment_plugin: Some(Version::parse("0.3.5").unwrap()),
            },
        )
        .unwrap_err();

        assert_eq!(error, UpdateError::InvalidSignature);
    }

    #[test]
    fn rejects_insecure_artifact_url() {
        let mut document = payload("0.3.6");
        document["components"][0]["artifact"]["url"] =
            serde_json::Value::String("http://updates.example.test/app.zip".to_owned());
        let (bytes, endpoint) = signed_manifest(document);

        let error = verify_manifest(
            &bytes,
            &endpoint,
            &InstalledComponentVersions {
                app: Version::parse("0.3.5").unwrap(),
                equipment_plugin: Some(Version::parse("0.3.5").unwrap()),
            },
        )
        .unwrap_err();

        assert_eq!(error, UpdateError::InvalidArtifactUrl);
    }

    #[test]
    fn managed_release_paths_exclude_user_data_and_traversal() {
        assert!(safe_managed_release_path(Path::new("nte-dps-tool.exe")));
        assert!(safe_managed_release_path(Path::new("plugins/dwmapi.dll")));
        assert!(!safe_managed_release_path(Path::new("config.json")));
        assert!(!safe_managed_release_path(Path::new("../nte-dps-tool.exe")));
        assert!(!safe_managed_release_path(Path::new(
            "plugins/dwmapi.dll:payload"
        )));
        assert!(!safe_managed_release_path(Path::new("plugins/CON")));
        assert!(!safe_managed_release_path(Path::new("res/data.json.")));
    }
}
