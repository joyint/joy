// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Which certificates the connector's own client trusts (D1.12, applied
//! to the connector by D2.8: "the client honours ... the same OS trust
//! store").
//!
//! The default is the operating system's own store, through rustls'
//! platform verifier: the Windows certificate store, the macOS system
//! anchors plus the Keychain trust settings, and on Linux the OpenSSL
//! style default paths, which is what `update-ca-certificates` and
//! `SSL_CERT_FILE` write. A corporate CA installed the normal way is
//! therefore trusted with no joy setting anywhere.
//!
//! The one escape hatch is joy's own and Linux only (decision 25):
//! `ca_bundle` / `ca_dir` in `forges.yaml`, and `http.sslCAInfo` /
//! `http.sslCAPath` from git config, because that is the setting a
//! corporate workstation image already carries. On macOS and Windows
//! those are refused with the sentence that names the system store,
//! because the engine cannot honour them there either
//! (`GIT_OPT_SET_SSL_CERT_LOCATIONS` is compiled for OpenSSL and
//! mbedTLS only).
//!
//! What joy will not do is listed in D1.12 and holds here: no
//! `http.sslVerify`, no way to turn verification off, no client
//! certificate.

use std::path::{Path, PathBuf};

use crate::config::Instance;
use crate::gitconfig::GitConfig;

/// What the client trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// The operating system's own store.
    Platform,
    /// Exactly these PEM files and directories, Linux only.
    Files {
        bundle: Option<PathBuf>,
        dir: Option<PathBuf>,
    },
}

/// The sentence a person reads when they set a CA file on an operating
/// system whose store joy cannot replace.
pub const FOREIGN_CA_SENTENCE: &str =
    "git's http.sslCAInfo does not apply here; install the certificate in the system keychain (macOS) or the Windows certificate store.";

/// The trust for one instance: the configured files where this
/// operating system can honour them, the platform store otherwise.
///
/// `report` receives the refusal sentence where one is due, so the
/// caller decides where it goes (the connector writes it on stderr).
pub fn decide(
    instance: Option<&Instance>,
    config: &GitConfig,
    report: &mut dyn FnMut(&str),
) -> Trust {
    let bundle = instance
        .and_then(|i| i.ca_bundle.clone())
        .or_else(|| config.get("http", "sslcainfo").map(PathBuf::from));
    let dir = instance
        .and_then(|i| i.ca_dir.clone())
        .or_else(|| config.get("http", "sslcapath").map(PathBuf::from));
    if bundle.is_none() && dir.is_none() {
        return Trust::Platform;
    }
    if !cfg!(target_os = "linux") {
        report(FOREIGN_CA_SENTENCE);
        return Trust::Platform;
    }
    Trust::Files { bundle, dir }
}

/// Every certificate in a bundle file and a directory of PEM files.
pub fn certificates(bundle: Option<&Path>, dir: Option<&Path>) -> Result<Vec<Vec<u8>>, String> {
    let mut ders: Vec<Vec<u8>> = Vec::new();
    if let Some(path) = bundle {
        ders.extend(read_pem(path)?);
    }
    if let Some(path) = dir {
        let entries = std::fs::read_dir(path)
            .map_err(|e| format!("{} is not readable: {e}", path.display()))?;
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "pem" | "crt" | "cer" | "0"))
            })
            .collect();
        files.sort();
        for file in files {
            ders.extend(read_pem(&file)?);
        }
    }
    if ders.is_empty() {
        return Err("no certificate was found in the configured CA bundle or directory".into());
    }
    Ok(ders)
}

/// The DER bodies of every CERTIFICATE block in a PEM file. A minimal
/// reader on purpose: the connector needs certificates out of a file a
/// distribution wrote, not a PEM library.
fn read_pem(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("{} is not readable: {e}", path.display()))?;
    let mut out = Vec::new();
    let mut base64 = String::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("-----BEGIN") && line.contains("CERTIFICATE") {
            inside = true;
            base64.clear();
            continue;
        }
        if line.starts_with("-----END") && line.contains("CERTIFICATE") {
            inside = false;
            out.push(decode_base64(&base64).ok_or_else(|| {
                format!(
                    "{} carries a certificate joy could not read",
                    path.display()
                )
            })?);
            base64.clear();
            continue;
        }
        if inside {
            base64.push_str(line);
        }
    }
    Ok(out)
}

/// Standard base64, no padding tricks: a PEM body and nothing else.
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits: u32 = 0;
    let mut have = 0;
    let mut out = Vec::new();
    for byte in input.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = TABLE.iter().position(|c| *c == byte)? as u32;
        bits = (bits << 6) | value;
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push(((bits >> have) & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(bundle: Option<&str>) -> Instance {
        Instance {
            host: "git.acme.test".into(),
            kind: "github".into(),
            ca_bundle: bundle.map(PathBuf::from),
            ..Instance::default()
        }
    }

    #[test]
    fn nothing_configured_is_the_platform_store() {
        let mut said = Vec::new();
        let trust = decide(None, &GitConfig::from_text(""), &mut |s| {
            said.push(s.to_string())
        });
        assert_eq!(trust, Trust::Platform);
        assert!(said.is_empty());
    }

    #[test]
    fn the_escape_hatch_is_linux_only_and_says_so_elsewhere() {
        let mut said = Vec::new();
        let trust = decide(
            Some(&instance(Some("/etc/pki/corp.pem"))),
            &GitConfig::from_text(""),
            &mut |s| said.push(s.to_string()),
        );
        if cfg!(target_os = "linux") {
            assert_eq!(
                trust,
                Trust::Files {
                    bundle: Some(PathBuf::from("/etc/pki/corp.pem")),
                    dir: None
                }
            );
            assert!(said.is_empty());
        } else {
            assert_eq!(trust, Trust::Platform);
            assert_eq!(said, vec![FOREIGN_CA_SENTENCE.to_string()]);
        }
    }

    #[test]
    fn git_configs_ca_keys_are_the_same_hatch() {
        let config = GitConfig::from_text("[http]\n sslCAInfo = /etc/pki/from-git.pem\n");
        let mut said = Vec::new();
        let trust = decide(None, &config, &mut |s| said.push(s.to_string()));
        if cfg!(target_os = "linux") {
            assert_eq!(
                trust,
                Trust::Files {
                    bundle: Some(PathBuf::from("/etc/pki/from-git.pem")),
                    dir: None
                }
            );
        } else {
            assert_eq!(trust, Trust::Platform);
        }
    }

    #[test]
    fn a_pem_bundle_yields_its_certificates() {
        let dir = std::env::temp_dir().join(format!("joy-trust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bundle.pem");
        // two blocks, the bodies are "Hi" and "Ho" base64-encoded
        std::fs::write(
            &path,
            "-----BEGIN CERTIFICATE-----\nSGk=\n-----END CERTIFICATE-----\n\
             -----BEGIN CERTIFICATE-----\nSG8=\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let certs = certificates(Some(&path), None).unwrap();
        assert_eq!(certs, vec![b"Hi".to_vec(), b"Ho".to_vec()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_bundle_is_an_error_and_not_an_empty_trust_store() {
        let dir = std::env::temp_dir().join(format!("joy-trust-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.pem");
        std::fs::write(&path, "# nothing here\n").unwrap();
        assert!(certificates(Some(&path), None).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
