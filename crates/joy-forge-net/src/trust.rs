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
//! those are refused with one sentence per entry that names the entry,
//! where it came from and the step that does work there, because the
//! engine cannot honour them there either
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

/// Where a refused entry came from, for the sentence that names it.
const FORGES_YAML: &str = "forges.yaml";
const GIT_CONFIG: &str = "git config";

/// The sentence a person reads when they set a CA location on an
/// operating system whose store joy cannot replace (D1.12).
///
/// It is word for word the sentence `joy_core::vcs::proxy` prints for
/// the same entry on the engine side, and its per OS step is the
/// `tls_untrusted` next step of D1.8c, so a person behind an
/// intercepting CA reads ONE instruction and not two, whichever half of
/// joy made the contact.
pub fn foreign_ca_sentence(key: &str, source: &str) -> String {
    // The connector ships for Linux, macOS and Windows and nothing else
    // (docs/plugins.md, "The size of the connector"), and Linux never
    // reaches this sentence, so the two stores that refuse are the
    // whole of the choice.
    let step = if cfg!(windows) {
        "your administrator must install the CA in the Windows certificate store"
    } else {
        "add your organisation's CA to the login or System keychain and mark it trusted"
    };
    format!(
        "joy ignores {key} from {source}: it does not apply here, because this system checks \
         certificates against its own store. To trust an internal CA, {step}."
    )
}

/// One configured CA location, with the key it was written under and
/// the place it came from, so that a refusal can name both (D1.12).
struct Configured {
    key: &'static str,
    source: &'static str,
    path: PathBuf,
}

/// The `forges.yaml` entry wins over the git config key, and whichever
/// answers keeps its own name.
fn configured(
    from_instance: Option<PathBuf>,
    instance_key: &'static str,
    from_git: Option<PathBuf>,
    git_key: &'static str,
) -> Option<Configured> {
    if let Some(path) = from_instance {
        return Some(Configured {
            key: instance_key,
            source: FORGES_YAML,
            path,
        });
    }
    from_git.map(|path| Configured {
        key: git_key,
        source: GIT_CONFIG,
        path,
    })
}

/// The trust for one instance: the configured files where this
/// operating system can honour them, the platform store otherwise.
///
/// `report` receives one refusal sentence per refused entry, so the
/// caller decides where they go (the connector writes them on stderr).
pub fn decide(
    instance: Option<&Instance>,
    config: &GitConfig,
    report: &mut dyn FnMut(&str),
) -> Trust {
    let bundle = configured(
        instance.and_then(|i| i.ca_bundle.clone()),
        "ca_bundle",
        config.get("http", "sslcainfo").map(PathBuf::from),
        "http.sslCAInfo",
    );
    let dir = configured(
        instance.and_then(|i| i.ca_dir.clone()),
        "ca_dir",
        config.get("http", "sslcapath").map(PathBuf::from),
        "http.sslCAPath",
    );
    if bundle.is_none() && dir.is_none() {
        return Trust::Platform;
    }
    if !cfg!(target_os = "linux") {
        // One sentence per entry and not one for the pair: somebody who
        // set two keys is told about both, by name.
        for entry in [&bundle, &dir].into_iter().flatten() {
            report(&foreign_ca_sentence(entry.key, entry.source));
        }
        return Trust::Platform;
    }
    Trust::Files {
        bundle: bundle.map(|entry| entry.path),
        dir: dir.map(|entry| entry.path),
    }
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
            assert_eq!(said, vec![foreign_ca_sentence("ca_bundle", "forges.yaml")]);
        }
    }

    // D1.12 quotes this sentence, and `joy_core::vcs::proxy::ca_refusal`
    // builds the same one for the same entry on the engine side. The
    // half of joy that refuses is not the half CI runs on, so the
    // wording is checked here on every operating system.
    #[test]
    fn the_refusal_names_the_entry_its_source_and_the_step_of_this_system() {
        let said = foreign_ca_sentence("http.sslCAInfo", "git config");
        assert!(
            said.starts_with(
                "joy ignores http.sslCAInfo from git config: it does not apply here, because \
                 this system checks certificates against its own store. To trust an internal \
                 CA, "
            ),
            "{said}"
        );
        if cfg!(windows) {
            assert!(
                said.ends_with(
                    "your administrator must install the CA in the Windows certificate store."
                ),
                "{said}"
            );
        } else {
            assert!(
                said.ends_with(
                    "add your organisation's CA to the login or System keychain and mark it \
                     trusted."
                ),
                "{said}"
            );
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
