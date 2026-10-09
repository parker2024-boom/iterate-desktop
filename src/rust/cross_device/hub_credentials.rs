//! Recover the existing current-user credential when an auto-started desktop
//! process did not inherit the Bridge launcher's source-token environment.
use super::{HubConfig, Result};
use serde::Deserialize;
use std::{io::Read, path::Path};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB},
};

#[derive(Deserialize)]
struct SavedCredential {
    version: u32,
    endpoint: String,
    device_id: String,
    token_env: String,
    token: String,
}

pub(super) fn source_token(config: &HubConfig) -> Result<String> {
    resolve(std::env::var(&config.token_env).ok(), || {
        let root = dirs::data_local_dir().ok_or("hub_source_credential_missing")?;
        load(&root.join("iterate/hub-source.dpapi"), config)
    })
}

fn resolve(environment: Option<String>, saved: impl FnOnce() -> Result<String>) -> Result<String> {
    match environment.filter(|token| !token.is_empty()) {
        Some(token) => Ok(token),
        None => saved(),
    }
}

fn load(path: &Path, config: &HubConfig) -> Result<String> {
    use std::os::windows::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "hub_source_credential_missing")?;
    if metadata.len() > 65536 || metadata.file_attributes() & 0x400 != 0 {
        return Err("hub_source_credential_invalid".into());
    }
    let mut encrypted = Vec::new();
    std::fs::File::open(path).and_then(|file| file.take(65537).read_to_end(&mut encrypted))
        .map_err(|_| "hub_source_credential_missing")?;
    if encrypted.is_empty() || encrypted.len() > 65536 {
        return Err("hub_source_credential_invalid".into());
    }
    let input = CRYPT_INTEGER_BLOB { cbData: encrypted.len() as u32, pbData: encrypted.as_mut_ptr() };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let saved = unsafe {
        if CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(),
            std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output) == 0 {
            return Err("hub_source_credential_invalid".into());
        }
        if output.pbData.is_null() {
            return Err("hub_source_credential_invalid".into());
        }
        let bytes = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let saved = serde_json::from_slice::<SavedCredential>(bytes);
        bytes.fill(0);
        LocalFree(output.pbData.cast());
        saved.map_err(|_| "hub_source_credential_invalid")?
    };
    if saved.version != 1 || saved.endpoint != config.endpoint || saved.device_id != config.device_id
        || saved.token_env != config.token_env || !(32..=512).contains(&saved.token.len())
        || !saved.token.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')) {
        return Err("hub_source_credential_binding_mismatch".into());
    }
    Ok(saved.token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use windows_sys::Win32::Security::Cryptography::CryptProtectData;

    fn config() -> HubConfig {
        HubConfig { endpoint: "https://hub.example.test".into(), device_id: "fixture-device".into(),
            token_env: "FIXTURE_HUB_TOKEN".into(), ca_certificate: None }
    }

    fn credential() -> Value {
        json!({"version":1,"endpoint":"https://hub.example.test","device_id":"fixture-device",
            "token_env":"FIXTURE_HUB_TOKEN","token":"synthetic-token-0123456789abcdef0123456789"})
    }

    fn protect(path: &Path, value: &Value) {
        protect_bytes(path, serde_json::to_vec(value).unwrap());
    }

    fn protect_bytes(path: &Path, mut bytes: Vec<u8>) {
        let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_mut_ptr() };
        let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
        unsafe {
            assert_ne!(CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(),
                std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output), 0);
            std::fs::write(path, std::slice::from_raw_parts(output.pbData, output.cbData as usize)).unwrap();
            LocalFree(output.pbData.cast());
        }
        bytes.fill(0);
    }

    #[test]
    fn current_user_saved_credential_recovers_without_inherited_environment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.dpapi");
        let value = credential();
        protect(&path, &value);
        for environment in [None, Some(String::new())] {
            assert_eq!(resolve(environment, || load(&path, &config())).unwrap(), value["token"]);
        }
    }

    #[test]
    fn explicit_environment_wins_without_reading_saved_credential() {
        assert_eq!(resolve(Some("explicit-token".into()), || panic!("must not read saved credential"))
            .unwrap(), "explicit-token");
    }

    #[test]
    fn saved_credential_rejects_other_routes_versions_and_invalid_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.dpapi");
        for (field, replacement) in [("version", json!(2)), ("endpoint", json!("https://other.test")),
            ("device_id", json!("other-device")), ("token_env", json!("OTHER_TOKEN")),
            ("token", json!("")), ("token", json!("x".repeat(513))),
            ("token", json!("invalid secret with spaces 0123456789abcdef"))] {
            let mut value = credential();
            value[field] = replacement;
            protect(&path, &value);
            assert!(load(&path, &config()).is_err(), "accepted mismatched {field}");
        }
    }

    #[test]
    fn missing_corrupt_and_oversized_files_fail_without_credentials_in_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.dpapi");
        assert!(load(&path, &config()).is_err());
        for content in [b"corrupt ciphertext".to_vec(), vec![0; 65537]] {
            std::fs::write(&path, content).unwrap();
            assert!(load(&path, &config()).unwrap_err().starts_with("hub_source_credential_"));
        }
    }

    #[test]
    fn empty_ciphertext_and_decrypted_invalid_json_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.dpapi");
        std::fs::write(&path, []).unwrap();
        assert_eq!(load(&path, &config()).unwrap_err(), "hub_source_credential_invalid");
        protect_bytes(&path, b"synthetic decrypted data that is not JSON".to_vec());
        assert_eq!(load(&path, &config()).unwrap_err(), "hub_source_credential_invalid");
    }

    #[test]
    fn reparse_point_is_rejected_before_opening() {
        use std::os::windows::fs::{symlink_file, MetadataExt};
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("valid.dpapi");
        let link = dir.path().join("redirect.dpapi");
        protect(&target, &credential());
        assert!(load(&target, &config()).is_ok());
        if symlink_file(&target, &link).is_err() {
            // A directory junction needs no symbolic-link privilege and still
            // exercises the final-path reparse rejection on restricted hosts.
            let result = std::process::Command::new("cmd.exe")
                .args(["/C", "mklink", "/J"]).arg(&link).arg(dir.path()).output().unwrap();
            assert!(result.status.success(), "could not create owned reparse fixture");
        }
        assert_ne!(std::fs::symlink_metadata(&link).unwrap().file_attributes() & 0x400, 0);
        assert_eq!(load(&link, &config()).unwrap_err(), "hub_source_credential_invalid");
    }
}
