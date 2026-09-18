// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use super::*;

/// github.com's ed25519 host key, as GitHub publishes it at
/// <https://api.github.com/meta>. Its fingerprint is the one on
/// GitHub's own fingerprint page.
const GITHUB_ED25519: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
const GITHUB_ED25519_FINGERPRINT: &str = "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU";

/// A second, different ed25519 key (codeberg.org's), for the mismatch
/// cases.
const OTHER_ED25519: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIIVIC02vnjFyL+I4RHfvIGNtOgJMe769VTF1VR4EB3ZB";

fn blob(base64: &str) -> Vec<u8> {
    decode_base64(base64).expect("a base64 key")
}

fn file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn the_fingerprint_is_the_string_the_forge_publishes() {
    assert_eq!(
        fingerprint(&blob(GITHUB_ED25519)),
        GITHUB_ED25519_FINGERPRINT
    );
    // unpadded, which is what ssh prints and what every forge page
    // shows
    assert!(!fingerprint(&blob(GITHUB_ED25519)).ends_with('='));
}

/// Lines `ssh-keyscan -H` really wrote, one for port 22 and one for the
/// bracketed form of port 443. They are the proof that joy's
/// `HMAC-SHA1(salt, name)` is OpenSSH's, including what OpenSSH hashes
/// for a port other than 22.
#[test]
fn a_hashed_entry_written_by_openssh_matches() {
    let dir = tempfile::tempdir().unwrap();
    let text = format!(
        "|1|7ZWcGIyZqgBWmYFfoNg30zmAu1A=|Q+V4oM3wjeGChFklw2DOSV7wxO4= ssh-ed25519 {GITHUB_ED25519}\n\
         |1|SfNJkQdRhRMfhHC/tT/21RBOe1Y=|z78sZw8/MunmETJBYwhj7Xsma44= ssh-ed25519 {GITHUB_ED25519}\n"
    );
    let path = file(dir.path(), "known_hosts", &text);
    let files = vec![path.clone()];
    assert_eq!(
        look_up(
            &files,
            "github.com",
            22,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::Known {
            file: path.clone(),
            line: 1
        }
    );
    assert_eq!(
        look_up(
            &files,
            "ssh.github.com",
            443,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::Known {
            file: path.clone(),
            line: 2
        }
    );
    // the same name on another port is another name, exactly as ssh
    // reads it
    assert_eq!(
        look_up(
            &files,
            "ssh.github.com",
            22,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::Unknown
    );
}

#[test]
fn a_pattern_matches_with_star_question_mark_and_negation() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(
        dir.path(),
        "known_hosts",
        &format!(
            "*.example.com,!build.example.com ssh-ed25519 {GITHUB_ED25519}\n\
             git?.internal ssh-ed25519 {GITHUB_ED25519}\n"
        ),
    );
    let files = vec![path];
    let key = blob(GITHUB_ED25519);
    for host in ["git.example.com", "GIT.EXAMPLE.COM", "git1.internal"] {
        assert!(
            matches!(
                look_up(&files, host, 22, "ssh-ed25519", &key),
                Verdict::Known { .. }
            ),
            "{host} should match"
        );
    }
    for host in ["build.example.com", "example.com", "git12.internal"] {
        assert_eq!(
            look_up(&files, host, 22, "ssh-ed25519", &key),
            Verdict::Unknown,
            "{host} should not match"
        );
    }
}

#[test]
fn the_bracketed_form_carries_the_port() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(
        dir.path(),
        "known_hosts",
        &format!("[git.example.com]:2222 ssh-ed25519 {GITHUB_ED25519}\n"),
    );
    let files = vec![path];
    let key = blob(GITHUB_ED25519);
    assert!(matches!(
        look_up(&files, "git.example.com", 2222, "ssh-ed25519", &key),
        Verdict::Known { .. }
    ));
    assert_eq!(
        look_up(&files, "git.example.com", 22, "ssh-ed25519", &key),
        Verdict::Unknown
    );
}

#[test]
fn a_changed_key_is_a_mismatch_that_names_the_file_and_the_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(
        dir.path(),
        "known_hosts",
        &format!(
            "# a comment\n\
             other.example.com ssh-ed25519 {OTHER_ED25519}\n\
             github.com ssh-ed25519 {OTHER_ED25519}\n"
        ),
    );
    let verdict = look_up(
        std::slice::from_ref(&path),
        "github.com",
        22,
        "ssh-ed25519",
        &blob(GITHUB_ED25519),
    );
    assert_eq!(
        verdict,
        Verdict::Mismatch {
            file: path,
            line: 3,
            expected: fingerprint(&blob(OTHER_ED25519)),
        }
    );
}

#[test]
fn a_host_with_lines_of_other_types_only_is_unknown_for_this_type() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(
        dir.path(),
        "known_hosts",
        &format!("github.com ecdsa-sha2-nistp256 {OTHER_ED25519}\n"),
    );
    assert_eq!(
        look_up(
            &[path],
            "github.com",
            22,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::UnknownKeyType {
            known_types: vec!["ecdsa-sha2-nistp256".to_string()]
        }
    );
}

#[test]
fn a_revoked_line_beats_a_matching_line_in_an_earlier_file() {
    let dir = tempfile::tempdir().unwrap();
    let user = file(
        dir.path(),
        "known_hosts",
        &format!("github.com ssh-ed25519 {GITHUB_ED25519}\n"),
    );
    let global = file(
        dir.path(),
        "ssh_known_hosts",
        &format!("@revoked github.com ssh-ed25519 {GITHUB_ED25519}\n"),
    );
    assert_eq!(
        look_up(
            &[user, global.clone()],
            "github.com",
            22,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::Revoked {
            file: global,
            line: 1
        }
    );
}

#[test]
fn a_revoked_line_about_another_key_says_nothing_about_this_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(
        dir.path(),
        "known_hosts",
        &format!("@revoked github.com ssh-ed25519 {OTHER_ED25519}\n"),
    );
    // not a mismatch either: the file holds no line that CLAIMS to be
    // this host's key
    assert_eq!(
        look_up(
            &[path],
            "github.com",
            22,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::Unknown
    );
}

#[test]
fn a_cert_authority_line_never_matches() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(
        dir.path(),
        "known_hosts",
        &format!("@cert-authority *.example.com ssh-ed25519 {GITHUB_ED25519}\n"),
    );
    assert_eq!(
        look_up(
            &[path],
            "git.example.com",
            22,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::Unknown
    );
}

#[test]
fn every_file_is_read_in_the_order_ssh_reads_them() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("known_hosts");
    let second = file(
        dir.path(),
        "known_hosts2",
        &format!("github.com ssh-ed25519 {GITHUB_ED25519}\n"),
    );
    // a missing file is not an error, it is a file with nothing in it
    assert_eq!(
        look_up(
            &[missing, second.clone()],
            "github.com",
            22,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::Known {
            file: second,
            line: 1
        }
    );
}

/// The rules are libssh2's, so every case here is one libssh2 really
/// refuses, and refusing the line means refusing the whole file.
#[test]
fn an_unparsable_line_is_reported_by_its_line_number() {
    let good = format!("github.com ssh-ed25519 {GITHUB_ED25519}");
    let cases: [(&str, &str); 4] = [
        ("nokey.example.com", "the line names a host and no key"),
        (
            "short.example.com ssh-rsa AAAA",
            "the key field is shorter than twenty characters",
        ),
        (
            "|1|not+base64+at+all|AAAAA ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA",
            "the hashed host field is not base64",
        ),
        (
            "|1|0123456789012345678901234567890123|AAAA ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA",
            "the salt of the hashed host field is too long",
        ),
    ];
    for (bad, reason) in cases {
        let text = format!("# a comment\n\n{good}\n{bad}\n{good}\n");
        assert_eq!(
            validate_text(&text),
            Some(Fault { line: 4, reason }),
            "{bad}"
        );
    }
    // a long host name is the fifth rule, and it needs a long line
    let long = format!("{} ssh-ed25519 {GITHUB_ED25519}", "a".repeat(255));
    assert_eq!(
        validate_text(&format!("{good}\n{long}\n")),
        Some(Fault {
            line: 2,
            reason: "a host name on the line is longer than 254 characters"
        })
    );
}

#[test]
fn a_file_openssh_wrote_is_parsable() {
    let text = format!(
        "# a comment, and a blank line follow\n\n\
         github.com ssh-ed25519 {GITHUB_ED25519}\n\
         |1|7ZWcGIyZqgBWmYFfoNg30zmAu1A=|Q+V4oM3wjeGChFklw2DOSV7wxO4= ssh-ed25519 {GITHUB_ED25519}\n\
         @revoked [git.example.com]:2222 ssh-ed25519 {OTHER_ED25519}\n\
         github.com ssh-ed25519 {GITHUB_ED25519} an old comment field\n"
    );
    assert_eq!(validate_text(&text), None);
    assert_eq!(parse(&text).len(), 4);
}

#[test]
fn the_line_joy_writes_is_the_line_joy_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".ssh").join("known_hosts");
    let key = blob(GITHUB_ED25519);
    for (host, port, hashed) in [
        ("github.com", 22u16, true),
        ("git.example.com", 2222, true),
        ("gitlab.com", 22, false),
    ] {
        let line = line_for(host, port, "ssh-ed25519", &key, hashed, None);
        assert_eq!(line.matches('\n').count(), 1, "one line, {host}");
        assert_eq!(hashed, line.starts_with("|1|"), "hashed, {host}");
        append(&path, &line).expect("the line is written");
        assert!(
            matches!(
                look_up(std::slice::from_ref(&path), host, port, "ssh-ed25519", &key),
                Verdict::Known { .. }
            ),
            "{host} is known after the append"
        );
        // and libssh2 could read the file joy wrote
        assert_eq!(
            validate_text(&std::fs::read_to_string(&path).unwrap()),
            None
        );
    }
    // appended, never rewritten
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
}

#[cfg(unix)]
#[test]
fn the_file_and_its_directory_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".ssh").join("known_hosts");
    let key = blob(GITHUB_ED25519);
    append(
        &path,
        &line_for("github.com", 22, "ssh-ed25519", &key, false, None),
    )
    .unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn a_certificate_host_key_is_named_by_its_own_blob() {
    // an OpenSSH host certificate names itself in its first field, and
    // git2 reports it as `SshHostKeyType::Unknown` with no name at all
    let mut blob = Vec::new();
    let name = b"ssh-ed25519-cert-v01@openssh.com";
    blob.extend((name.len() as u32).to_be_bytes());
    blob.extend(name);
    blob.extend(b"the rest of a certificate");
    let named = key_type_in_blob(&blob).expect("the blob names itself");
    assert_eq!(named, "ssh-ed25519-cert-v01@openssh.com");
    assert!(is_certificate_type(&named));
    assert!(!is_certificate_type("ssh-ed25519"));
    assert_eq!(
        key_type_in_blob(&super::decode_base64(GITHUB_ED25519).unwrap()).as_deref(),
        Some("ssh-ed25519")
    );
    // a blob that names nothing readable is not a type
    assert_eq!(key_type_in_blob(&[0, 0, 0, 200, 1, 2]), None);
    assert_eq!(key_type_in_blob(&[]), None);
}

/// The data half of the pin file: every blob it records really is the
/// key whose fingerprint the forge publishes, so a pin can never turn
/// into "joy refuses the forge's own key". The fingerprints are the
/// ones the forges' own pages carry; `just check-host-key-pins` takes
/// the keys off the forges again and checks the same equality against
/// the live pages, and the nightly CI job runs it.
#[test]
fn pins_match_the_published_fingerprints() {
    let hosts = pins::recorded();
    let names: Vec<&str> = hosts.iter().map(|pin| pin.host.as_str()).collect();
    assert_eq!(
        names,
        [
            "github.com",
            "ssh.github.com",
            "gitlab.com",
            "altssh.gitlab.com",
            "codeberg.org"
        ]
    );
    for pin in hosts {
        assert!(
            pin.published_at.starts_with("https://"),
            "{}: the page that publishes the fingerprints is recorded",
            pin.host
        );
        assert!(
            pin.source.starts_with("https://"),
            "{}: the URL the blobs came from is recorded",
            pin.host
        );
        // A pin carries the day it was taken, because that is the first
        // thing to look at when a pinned host starts refusing.
        assert!(
            chrono::NaiveDate::parse_from_str(&pin.taken, "%Y-%m-%d").is_ok(),
            "{}: the day the blobs were taken is recorded, and {:?} is not one",
            pin.host,
            pin.taken
        );
        assert!(
            pin.keys.len() >= 3,
            "{}: every published type is pinned",
            pin.host
        );
        for key in &pin.keys {
            let blob = key.blob().expect("a pinned key is base64");
            assert_eq!(
                fingerprint(&blob),
                key.fingerprint,
                "{} {}: the blob is the key the fingerprint names",
                pin.host,
                key.key_type
            );
            assert_eq!(
                key_type_in_blob(&blob).as_deref(),
                Some(key.key_type.as_str()),
                "{} {}: the blob names its own type",
                pin.host,
                key.key_type
            );
        }
    }
    // github.com's ed25519 fingerprint, as GitHub publishes it
    let github = pins::recorded_for("github.com").expect("github.com is pinned");
    assert_eq!(
        github
            .keys
            .iter()
            .find(|k| k.key_type == "ssh-ed25519")
            .map(|k| k.fingerprint.as_str()),
        Some(GITHUB_ED25519_FINGERPRINT)
    );
    // the alternate ssh endpoints carry the same keys as their host
    for (main, alternate) in [
        ("github.com", "ssh.github.com"),
        ("gitlab.com", "altssh.gitlab.com"),
    ] {
        let main: Vec<&str> = pins::recorded_for(main)
            .unwrap()
            .keys
            .iter()
            .map(|k| k.key.as_str())
            .collect();
        let alternate: Vec<&str> = pins::recorded_for(alternate)
            .unwrap()
            .keys
            .iter()
            .map(|k| k.key.as_str())
            .collect();
        assert_eq!(main, alternate);
    }
}

/// The operator answered decision 23, so the pin file a release ships
/// carries the three public forges and a contact may consult them.
/// `shipped` reads the release the running binary belongs to and
/// `recorded` reads the file in this source tree; in a test binary,
/// which belongs to no release, the two are the same file, and this is
/// the test that would notice if they stopped being.
#[test]
fn the_pin_file_a_release_ships_carries_the_three_public_forges() {
    let shipped: Vec<&str> = pins::shipped()
        .iter()
        .map(|pin| pin.host.as_str())
        .collect();
    let recorded: Vec<&str> = pins::recorded()
        .iter()
        .map(|pin| pin.host.as_str())
        .collect();
    assert_eq!(shipped, recorded);
    let github = pins::consulted_for("github.com").expect("github.com is pinned");
    assert_eq!(github.forge, "GitHub");
    assert!(github.keys.iter().any(|key| key.key_type == "ssh-ed25519"));
    // and a host nobody pinned is still a host with no pin
    assert!(pins::consulted_for("forge.example.com").is_none());
}

/// D1.4a asks for the pins as DATA in the release: a key rotation at a
/// pinned forge has to be a file that is replaced, never a joy that is
/// rebuilt. So the file is looked for in the release, in both layouts
/// the installers produce, and the compiled-in copy answers only where
/// a binary stands alone.
#[test]
fn the_pin_file_is_read_from_the_release_and_not_only_from_the_binary() {
    let dir = Path::new("/opt/joy/bin");
    assert_eq!(
        pins::candidates(dir),
        vec![
            PathBuf::from("/opt/joy/bin/host-keys.json"),
            PathBuf::from("/opt/joy/share/joy/host-keys.json"),
        ]
    );
    // a binary with no directory above it still has the one beside it
    assert_eq!(
        pins::candidates(Path::new("/")),
        vec![PathBuf::from("/host-keys.json")]
    );
    // and a file that is really there is really read, which is what
    // makes a key rotation at a pinned forge a file that is replaced
    let release = tempfile::tempdir().unwrap();
    let bin = release.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    assert!(pins::file_beside(&bin).is_none());
    let share = release.path().join("share").join("joy");
    std::fs::create_dir_all(&share).unwrap();
    let one = r#"{"hosts": [{"host": "forge.example.com", "port": 22, "forge": "Example",
        "source": "https://example.com/meta", "published_at": "https://example.com/keys",
        "taken": "2026-09-18", "keys": []}]}"#;
    std::fs::write(share.join("host-keys.json"), one).unwrap();
    let (path, text) = pins::file_beside(&bin).expect("the release carries a pin file");
    assert_eq!(path, share.join("host-keys.json"));
    let hosts = pins::hosts_in(&text, "a test release");
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].host, "forge.example.com");
    // the copy beside the binary wins over the one in the share
    // directory, because that is the one an installer drops
    std::fs::write(bin.join("host-keys.json"), r#"{"hosts": []}"#).unwrap();
    let (path, text) = pins::file_beside(&bin).expect("the release carries a pin file");
    assert_eq!(path, bin.join("host-keys.json"));
    assert!(pins::hosts_in(&text, "a test release").is_empty());
    // and a pin file joy cannot read leaves joy without pins, never
    // without a contact
    assert!(pins::hosts_in("{ not json", "a broken pin file").is_empty());
}

/// The file libgit2 reads by itself, which is the file the
/// pre-validation is about: `known_hosts` beside the person's ssh
/// config, and nothing the config renamed.
#[test]
fn the_pre_validated_file_is_the_one_libgit2_reads() {
    let Some(path) = user_file() else {
        // no home directory on this machine: nothing to validate
        return;
    };
    assert!(
        path.ends_with(Path::new(".ssh").join("known_hosts")),
        "{path:?}"
    );
    assert_eq!(
        path.parent(),
        super::super::ssh_config::user_config_path()
            .as_deref()
            .and_then(Path::parent)
    );
    // reading it twice is one read: the file is validated once per
    // process, before the first ssh contact
    assert_eq!(user_file_refusal(), user_file_refusal());
}

/// What a person reads when libssh2 would refuse their file: the file,
/// the line number, and why every ssh contact fails until it is fixed.
#[test]
fn the_broken_file_sentence_names_the_file_and_the_line() {
    let path = PathBuf::from("/home/troi/.ssh/known_hosts");
    let good = format!("github.com ssh-ed25519 {GITHUB_ED25519}");
    assert_eq!(refusal_for(&path, format!("{good}\n").as_bytes()), None);
    let sentence = refusal_for(&path, format!("{good}\nbroken.example.com\n").as_bytes())
        .expect("the second line is refused");
    assert!(
        sentence.starts_with("/home/troi/.ssh/known_hosts line 2: "),
        "{sentence}"
    );
    assert!(sentence.contains("names a host and no key"), "{sentence}");
    assert!(sentence.contains("every ssh contact fails"), "{sentence}");
}

/// The pre-validation walks BYTES. A host field of at most two
/// characters sends libssh2 into its hashed branch, which reads the
/// salt out of the key field, so joy reads from the line's fourth byte
/// on; slicing a `&str` there panics when a character straddles the
/// index, and that panic would happen inside the `OnceLock` the
/// transport guard reads before every ssh contact: the contact dies and
/// the cell stays empty for the next one. The line below is the
/// smallest real shape of it.
#[test]
fn a_line_whose_key_field_is_not_ascii_is_reported_and_never_panics() {
    // libssh2 finds no separator in this one, stores nothing and reads
    // on, so the file is fine and the answer is that there is no fault
    let no_separator = "a \u{e9}aaaaaaaaaaaaaaaaaaaaaaaa\n";
    assert_eq!(validate_text(no_separator), None);
    // and this one it does refuse, with the byte that is not a
    // character boundary sitting in the salt
    let malformed = "a \u{e9}|aaaaaaaaaaaaaaaaaaaaaaaa\n";
    assert_eq!(
        validate_text(malformed),
        Some(Fault {
            line: 1,
            reason: "the hashed host field is malformed"
        })
    );
    // among good lines it is still line 2 and nothing else
    let good = format!("github.com ssh-ed25519 {GITHUB_ED25519}");
    assert_eq!(
        validate_text(&format!("{good}\n{malformed}{good}\n")).map(|f| f.line),
        Some(2)
    );
    assert_eq!(
        validate_text(&format!("{good}\n{no_separator}{good}\n")),
        None
    );
    // and a multi-byte comment on an ordinary line is no fault at all
    assert_eq!(
        validate_text(&format!("{good} schl\u{fc}ssel von zara\n")),
        None
    );
    // bytes that are not UTF-8 at all reach the same rules
    let mut raw = format!("{good} ").into_bytes();
    raw.extend_from_slice(&[0xff, 0xfe, b'\n']);
    assert_eq!(validate_bytes(&raw), None);
}

/// A known_hosts file is bytes, not a Rust string: a comment field
/// copied out of a key file can carry anything. libssh2 and OpenSSH
/// read such a file, so joy reads it too. Dropping it would make joy
/// call a host unknown that the file names, which a `Background` host
/// then refuses.
#[test]
fn a_file_with_a_byte_that_is_not_utf8_is_still_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known_hosts");
    let mut bytes = format!("github.com ssh-ed25519 {GITHUB_ED25519} zara").into_bytes();
    // a latin-1 u-umlaut, as a key file's comment would carry it
    bytes.extend_from_slice(&[0xfc, b'r', b'\n']);
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(
        look_up(
            std::slice::from_ref(&path),
            "github.com",
            22,
            "ssh-ed25519",
            &blob(GITHUB_ED25519)
        ),
        Verdict::Known {
            file: path.clone(),
            line: 1
        }
    );
}
