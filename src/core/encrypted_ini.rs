use std::path::Path;

use aes::Aes256;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::storage::io_util::atomic_write_text;

pub const ENCRYPTED_INI_MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncryptedIniError {
    TooLarge,
    ReadFailed,
    InvalidCiphertext,
    PlaintextTooLarge,
    WriteFailed,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum EncryptedIniKey {
    #[default]
    Global,
    China,
}

impl EncryptedIniKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::China => "china",
        }
    }

    pub fn key(self) -> &'static [u8; 32] {
        match self {
            Self::Global => b"UVbP6pjjw5KZhvddie3tfhg1pVkkveY8",
            Self::China => b"1zh6IOlIohrR88UNPjiLisrkWACUQYuz",
        }
    }

    pub fn all() -> [Self; 2] {
        [Self::Global, Self::China]
    }
}

#[derive(Default)]
pub struct EncryptedIniRecord {
    pub encrypted_line: String,
    payload_parts: Vec<String>,
    visible_parts: Vec<String>,
}

pub struct EncryptedIniDocument {
    key: EncryptedIniKey,
    plaintext: String,
    records: Vec<EncryptedIniRecord>,
    line_ending: String,
    final_newline: bool,
}

impl EncryptedIniDocument {
    pub fn key(&self) -> EncryptedIniKey {
        self.key
    }

    pub fn plaintext(&self) -> &str {
        &self.plaintext
    }

    pub fn encrypted_line_count(&self) -> usize {
        self.records.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncryptedIniSaveOutcome {
    Saved,
    Unchanged,
}

pub fn load_encrypted_ini_document(path: &Path) -> Result<EncryptedIniDocument, EncryptedIniError> {
    let metadata = std::fs::metadata(path).map_err(|_| EncryptedIniError::ReadFailed)?;
    if metadata.len() > ENCRYPTED_INI_MAX_BYTES {
        return Err(EncryptedIniError::TooLarge);
    }
    let encrypted = std::fs::read_to_string(path).map_err(|_| EncryptedIniError::ReadFailed)?;
    let (key, plaintext, records, line_ending, final_newline) =
        parse_encrypted_ini_text(&encrypted).map_err(|_| EncryptedIniError::InvalidCiphertext)?;
    if plaintext.len() as u64 > ENCRYPTED_INI_MAX_BYTES {
        return Err(EncryptedIniError::TooLarge);
    }
    Ok(EncryptedIniDocument {
        key,
        plaintext,
        records,
        line_ending,
        final_newline,
    })
}

pub fn save_encrypted_ini_document(
    path: &Path,
    document: &mut EncryptedIniDocument,
    plaintext: String,
    key: EncryptedIniKey,
) -> Result<EncryptedIniSaveOutcome, EncryptedIniError> {
    if plaintext.len() as u64 > ENCRYPTED_INI_MAX_BYTES {
        return Err(EncryptedIniError::PlaintextTooLarge);
    }
    if plaintext == document.plaintext && key == document.key {
        return Ok(EncryptedIniSaveOutcome::Unchanged);
    }
    let encrypted = encrypt_encrypted_ini_records(
        &plaintext,
        key,
        document.key,
        &document.records,
        &document.line_ending,
        document.final_newline,
    )
    .map_err(|_| EncryptedIniError::InvalidCiphertext)?;
    let (saved_key, saved_plaintext, records, line_ending, final_newline) =
        parse_encrypted_ini_text(&encrypted).map_err(|_| EncryptedIniError::InvalidCiphertext)?;
    let saved = EncryptedIniDocument {
        key: saved_key,
        plaintext: saved_plaintext,
        records,
        line_ending,
        final_newline,
    };
    atomic_write_text(path, &encrypted).map_err(|_| EncryptedIniError::WriteFailed)?;
    *document = saved;
    Ok(EncryptedIniSaveOutcome::Saved)
}

pub fn parse_encrypted_ini_text(
    text: &str,
) -> Result<
    (
        EncryptedIniKey,
        String,
        Vec<EncryptedIniRecord>,
        String,
        bool,
    ),
    String,
> {
    let mut active_key = EncryptedIniKey::Global;
    let mut output = Vec::new();
    let mut records = Vec::new();
    let line_ending = if text.contains("\r\n") {
        "\r\n".to_owned()
    } else {
        "\n".to_owned()
    };
    let final_newline = text.ends_with('\n') || text.ends_with('\r');
    for original in text.trim_start_matches('\u{feff}').lines() {
        let line = original.trim();
        if line.is_empty() {
            records.push(EncryptedIniRecord {
                encrypted_line: original.to_owned(),
                payload_parts: Vec::new(),
                visible_parts: Vec::new(),
            });
            continue;
        }
        if let Some((key, decrypted)) = decrypt_encrypted_ini_line(line)? {
            active_key = key;
            let payload_parts = decrypted
                .split("|SPLIT|")
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let visible_parts = payload_parts
                .iter()
                .filter(|part| !part.is_empty())
                .cloned()
                .collect::<Vec<_>>();
            output.extend(visible_parts.iter().cloned());
            records.push(EncryptedIniRecord {
                encrypted_line: original.to_owned(),
                payload_parts,
                visible_parts,
            });
        } else {
            output.push(original.to_owned());
            records.push(EncryptedIniRecord {
                encrypted_line: original.to_owned(),
                payload_parts: vec![original.to_owned()],
                visible_parts: vec![original.to_owned()],
            });
        }
    }
    Ok((
        active_key,
        output.join("\n"),
        records,
        line_ending,
        final_newline,
    ))
}

fn decrypt_encrypted_ini_line(line: &str) -> Result<Option<(EncryptedIniKey, String)>, String> {
    let Ok(encrypted) = BASE64.decode(line) else {
        return Ok(None);
    };
    if encrypted.is_empty() || encrypted.len() % 16 != 0 {
        return Ok(None);
    }
    for key in EncryptedIniKey::all() {
        let decrypted = decrypt_aes256_ecb(&encrypted, key.key())?;
        let Ok(unpadded) = pkcs7_unpad(&decrypted) else {
            continue;
        };
        let Ok(text) = String::from_utf8(unpadded.to_vec()) else {
            continue;
        };
        return Ok(Some((key, text)));
    }
    Ok(None)
}

#[cfg(test)]
pub fn encrypt_encrypted_ini_text(text: &str, key: EncryptedIniKey) -> Result<String, String> {
    let mut output = String::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let encrypted = encrypt_aes256_ecb(&pkcs7_pad(line.as_bytes()), key.key())?;
        output.push_str(&BASE64.encode(encrypted));
        output.push('\n');
    }
    Ok(output)
}

pub fn encrypt_encrypted_ini_records(
    text: &str,
    key: EncryptedIniKey,
    original_key: EncryptedIniKey,
    records: &[EncryptedIniRecord],
    line_ending: &str,
    final_newline: bool,
) -> Result<String, String> {
    let mut output_lines = Vec::new();
    let lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let mut line_index = 0;
    for record in records {
        let visible_count = record.visible_parts.len();
        if visible_count == 0 {
            output_lines.push(record.encrypted_line.clone());
            continue;
        }
        if line_index + visible_count > lines.len() {
            break;
        }
        let current_parts = &lines[line_index..line_index + visible_count];
        if key == original_key && current_parts == record.visible_parts.as_slice() {
            output_lines.push(record.encrypted_line.clone());
        } else {
            let mut payload_parts = record.payload_parts.clone();
            let mut current_index = 0;
            for part in &mut payload_parts {
                if !part.is_empty() {
                    *part = current_parts[current_index].clone();
                    current_index += 1;
                }
            }
            let payload = payload_parts.join("|SPLIT|");
            let encrypted = encrypt_aes256_ecb(&pkcs7_pad(payload.as_bytes()), key.key())?;
            let encrypted_line = BASE64.encode(encrypted);
            output_lines.push(encrypted_line);
        }
        line_index += visible_count;
    }
    for line in lines
        .iter()
        .skip(line_index)
        .filter(|line| !line.trim().is_empty())
    {
        let encrypted = encrypt_aes256_ecb(&pkcs7_pad(line.as_bytes()), key.key())?;
        let encrypted_line = BASE64.encode(encrypted);
        output_lines.push(encrypted_line);
    }
    let mut output = output_lines.join(line_ending);
    if final_newline {
        output.push_str(line_ending);
    }
    Ok(output)
}

fn decrypt_aes256_ecb(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    if !data.len().is_multiple_of(16) {
        return Err("AES 密文长度不是 16 字节块的整数倍".to_owned());
    }
    let cipher = Aes256::new_from_slice(key).map_err(|error| error.to_string())?;
    let mut output = data.to_vec();
    for block in output.as_chunks_mut::<16>().0 {
        cipher.decrypt_block(block.into());
    }
    Ok(output)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn encrypt_aes256_ecb(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    if !data.len().is_multiple_of(16) {
        return Err("AES 明文长度不是 16 字节块的整数倍".to_owned());
    }
    let cipher = Aes256::new_from_slice(key).map_err(|error| error.to_string())?;
    let mut output = data.to_vec();
    for block in output.as_chunks_mut::<16>().0 {
        cipher.encrypt_block(block.into());
    }
    Ok(output)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn pkcs7_pad(data: &[u8]) -> Vec<u8> {
    let padding = 16 - data.len() % 16;
    let mut output = Vec::with_capacity(data.len() + padding);
    output.extend_from_slice(data);
    output.extend(std::iter::repeat_n(padding as u8, padding));
    output
}

fn pkcs7_unpad(data: &[u8]) -> Result<&[u8], String> {
    let Some(&padding) = data.last() else {
        return Err("空数据无法移除 PKCS#7 padding".to_owned());
    };
    let padding = usize::from(padding);
    if padding == 0 || padding > 16 || padding > data.len() {
        return Err("PKCS#7 padding 无效".to_owned());
    }
    if !data[data.len() - padding..]
        .iter()
        .all(|byte| usize::from(*byte) == padding)
    {
        return Err("PKCS#7 padding 不一致".to_owned());
    }
    Ok(&data[..data.len() - padding])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("nte-encrypted-ini-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&directory).expect("create test directory");
        directory.join("Engine.ini")
    }

    #[test]
    fn document_save_round_trips_and_preserves_line_endings() {
        let path = test_path("round-trip");
        let encrypted =
            encrypt_encrypted_ini_text("[/Script/Test]\nValue=1", EncryptedIniKey::China)
                .expect("encrypt fixture")
                .replace('\n', "\r\n");
        std::fs::write(&path, encrypted).expect("write fixture");

        let mut document = load_encrypted_ini_document(&path).expect("load document");
        assert_eq!(document.key(), EncryptedIniKey::China);
        assert_eq!(document.plaintext(), "[/Script/Test]\nValue=1");
        assert_eq!(
            save_encrypted_ini_document(
                &path,
                &mut document,
                "[/Script/Test]\nValue=2".to_owned(),
                EncryptedIniKey::Global,
            ),
            Ok(EncryptedIniSaveOutcome::Saved)
        );
        let saved = std::fs::read_to_string(&path).expect("read saved fixture");
        assert!(saved.contains("\r\n"));
        let reloaded = load_encrypted_ini_document(&path).expect("reload document");
        assert_eq!(reloaded.key(), EncryptedIniKey::Global);
        assert_eq!(reloaded.plaintext(), "[/Script/Test]\nValue=2");
    }

    #[test]
    fn unchanged_document_keeps_original_ciphertext() {
        let path = test_path("unchanged");
        let encrypted = encrypt_encrypted_ini_text("Value=1", EncryptedIniKey::Global)
            .expect("encrypt fixture");
        std::fs::write(&path, &encrypted).expect("write fixture");
        let mut document = load_encrypted_ini_document(&path).expect("load document");

        assert_eq!(
            save_encrypted_ini_document(
                &path,
                &mut document,
                "Value=1".to_owned(),
                EncryptedIniKey::Global,
            ),
            Ok(EncryptedIniSaveOutcome::Unchanged)
        );
        assert_eq!(
            std::fs::read_to_string(path).expect("read fixture"),
            encrypted
        );
    }

    #[test]
    fn oversized_ciphertext_is_rejected_before_reading() {
        let path = test_path("too-large");
        let file = std::fs::File::create(&path).expect("create fixture");
        file.set_len(ENCRYPTED_INI_MAX_BYTES + 1)
            .expect("extend fixture");
        assert_eq!(
            load_encrypted_ini_document(&path).err(),
            Some(EncryptedIniError::TooLarge)
        );
    }
}
