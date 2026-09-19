// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::embedded::{self, EmbeddedFile};
use crate::error::JoyError;
use crate::host::HostKind;
use crate::model::project::{derive_acronym, Project};
use crate::store;
use crate::vcs::{default_vcs, Vcs};

pub const HOOK_FILES: &[EmbeddedFile] = &[
    EmbeddedFile {
        content: include_str!("../data/hooks/commit-msg"),
        target: "hooks/commit-msg",
        executable: true,
    },
    EmbeddedFile {
        content: include_str!("../data/hooks/prepare-commit-msg"),
        target: "hooks/prepare-commit-msg",
        executable: true,
    },
    // The tail of every joy hook (design D3.5): joy owns
    // `core.hooksPath`, so it runs what was there before.
    EmbeddedFile {
        content: include_str!("../data/hooks/joy-chain"),
        target: "hooks/joy-chain",
        executable: true,
    },
];

/// The hook path joy owns, as `core.hooksPath` spells it: relative to
/// the working tree, so it is the same value in every clone.
pub const JOY_HOOKS_PATH: &str = ".joy/hooks";

/// Where joy remembers the hook path it replaced (design D3.5).
///
/// A file joy writes, not a git config key, so it travels with the
/// checkout of the person who has it and never with the team:
/// `.joy/hooks/` is in the managed gitignore block.
pub const CHAINED_PATH_FILE: &str = "hooks/chained-path";

pub const CONFIG_FILES: &[EmbeddedFile] = &[EmbeddedFile {
    content: include_str!("../data/config.defaults.yaml"),
    target: "config.defaults.yaml",
    executable: false,
}];

pub const PROJECT_FILES: &[EmbeddedFile] = &[EmbeddedFile {
    content: include_str!("../data/project.defaults.yaml"),
    target: "project.defaults.yaml",
    executable: false,
}];

/// Dead pre-ADR-024 artefacts under `.joy/`. Before ADR-024 the AI
/// integration synced intermediate instruction/skill/capability files
/// into `.joy/ai/` and `.joy/capabilities/`; today every template is
/// embedded in the binary and rendered straight into the tool
/// directories (`.claude/`, `.qwen/`, `AGENTS.md`, `.github/`). The
/// current CLI neither reads nor writes these files, but AI tools that
/// stumble over them in an old repo treat their content as authoritative.
/// `joy update` and `joy ai init` remove them.
///
/// Paths are relative to the project root. Directories are removed
/// recursively. The current runtime data under `.joy/ai/agents/` is
/// deliberately NOT listed and stays untouched; legacy `.joy/ai/jobs/`
/// records are handled by the 2026-07 repo migration instead
/// (JOY-0207-DC), which stages their deletion.
pub const LEGACY_AI_ARTIFACTS: &[&str] = &[
    ".joy/ai/instructions.md",
    ".joy/ai/instructions",
    ".joy/ai/skills",
    ".joy/capabilities",
];

pub struct InitOptions {
    pub root: PathBuf,
    pub name: Option<String>,
    pub acronym: Option<String>,
    /// The founder's address, named by the host (`joy init --user`, the
    /// desktop's setup mask). Git config is only a prefill behind it.
    pub user: Option<String>,
    /// Project language code (ISO 639-1, e.g. "en", "de"). Defaults to "en".
    pub language: Option<String>,
    /// Who is behind this process. Decided by the host, never here (D1.1).
    pub host: HostKind,
    /// How an [`HostKind::Interactive`] host asks for the founder's address
    /// when neither `user` nor git config knows one. `None` means "do not
    /// ask", which is what every background host passes.
    pub ask: Option<Box<dyn AskFounderAddress>>,
}

impl InitOptions {
    /// A project in `root` with everything else left to the defaults: no
    /// name, no acronym, no address, a background host that asks nobody.
    pub fn new(root: PathBuf) -> Self {
        InitOptions {
            root,
            ..InitOptions::default()
        }
    }
}

impl Default for InitOptions {
    /// Everything unset and a background host, with an EMPTY root that
    /// the caller is expected to replace ([`InitOptions::new`] does).
    ///
    /// It exists so an out-of-repo caller (the desktop's
    /// `joy_init_project`) can write `InitOptions { root, user, ..
    /// Default::default() }` and keep compiling when this struct grows a
    /// field, instead of breaking on the next joy-core bump.
    fn default() -> Self {
        InitOptions {
            root: PathBuf::new(),
            name: None,
            acronym: None,
            user: None,
            language: None,
            host: HostKind::default(),
            ask: None,
        }
    }
}

/// How a host asks a person for the founding address (D3.9). Implemented
/// by the CLI over the terminal and by the desktop over its setup mask;
/// a test implements it over a scripted answer.
pub trait AskFounderAddress {
    /// The address the person typed, or `None` when they gave none.
    fn ask_founder_address(&mut self) -> Result<Option<String>, JoyError>;

    /// Tell the person why the address they just typed was not taken, so
    /// the next call can ask again. Only the forge alias guard produces
    /// one of these: a typo in the shape is caught inside the ask itself.
    /// The default says nothing, which is right for a mask that shows the
    /// refusal on its own.
    fn reject_founder_address(&mut self, _reason: &str) -> Result<(), JoyError> {
        Ok(())
    }
}

/// How often a person may answer the founding question before joy gives
/// up on this run. The same three the shape check inside the ask allows.
const FOUNDER_ASK_TRIES: u32 = 3;

/// The terminal ask: two sentences and one line of input. Generic over
/// its reader and writer so a test drives it with a fake stdin.
///
/// The question goes to the WRITER (the CLI passes stderr), so `joy init`
/// keeps one thing on stdout.
pub struct TerminalAsk<R: BufRead, W: Write> {
    input: R,
    output: W,
    tries: u32,
    /// Whether the two opening sentences were already written. They say
    /// why the question is being asked, which is worth saying once and
    /// tiresome to repeat when a rejected answer brings the person back.
    introduced: bool,
}

impl<R: BufRead, W: Write> TerminalAsk<R, W> {
    pub fn new(input: R, output: W) -> Self {
        TerminalAsk {
            input,
            output,
            tries: FOUNDER_ASK_TRIES,
            introduced: false,
        }
    }
}

impl TerminalAsk<std::io::BufReader<std::io::Stdin>, std::io::Stderr> {
    /// The ask a person typing `joy init` sees.
    pub fn stdio() -> Self {
        TerminalAsk::new(std::io::BufReader::new(std::io::stdin()), std::io::stderr())
    }
}

impl<R: BufRead, W: Write> AskFounderAddress for TerminalAsk<R, W> {
    fn ask_founder_address(&mut self) -> Result<Option<String>, JoyError> {
        let io = |e: std::io::Error| JoyError::Git(format!("cannot ask for the address: {e}"));
        if !self.introduced {
            self.introduced = true;
            writeln!(
                self.output,
                "This project does not know who you are yet, and git config names nobody."
            )
            .map_err(io)?;
            writeln!(
                self.output,
                "Your address becomes the founding member of this project."
            )
            .map_err(io)?;
        }
        for _ in 0..self.tries {
            write!(self.output, "Address: ").map_err(io)?;
            self.output.flush().map_err(io)?;
            let mut line = String::new();
            if self.input.read_line(&mut line).map_err(io)? == 0 {
                // stdin closed: nobody is answering after all.
                return Ok(None);
            }
            let answer = line.trim();
            if answer.is_empty() {
                return Ok(None);
            }
            if looks_like_an_address(answer) {
                return Ok(Some(answer.to_string()));
            }
            writeln!(self.output, "An address looks like you@example.com.").map_err(io)?;
        }
        Ok(None)
    }

    fn reject_founder_address(&mut self, reason: &str) -> Result<(), JoyError> {
        writeln!(self.output, "{reason}")
            .map_err(|e| JoyError::Git(format!("cannot ask for the address: {e}")))
    }
}

/// The shape check the ask applies before it accepts a typed answer: one
/// `@`, something on both sides, no whitespace. Deliberately not an RFC
/// 5322 parser; it catches a typo, it does not judge an address.
fn looks_like_an_address(candidate: &str) -> bool {
    let Some((local, domain)) = candidate.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !candidate.chars().any(char::is_whitespace)
}

#[derive(Debug)]
pub struct InitResult {
    pub project_dir: PathBuf,
    pub git_initialized: bool,
    pub git_existed: bool,
    /// The founding member this init registered. The caller's next step
    /// (setting up authentication) takes it from here instead of asking
    /// git config a second time (D3.9).
    pub founder: String,
}

pub struct OnboardResult {
    pub hooks_installed: bool,
    pub hooks_already_set: bool,
    /// The hook path joy took over and now chains to, when there was
    /// one (design D3.5). `None` when `core.hooksPath` was unset, in
    /// which case joy's hooks chain to git's own `$GIT_DIR/hooks`.
    pub chained: Option<String>,
}

pub fn init(options: InitOptions) -> Result<InitResult, JoyError> {
    let mut options = options;
    let root_dir = options.root.clone();
    let root = root_dir.as_path();
    let joy_dir = store::joy_dir(root);

    if store::is_initialized(root) {
        return Err(JoyError::AlreadyInitialized(joy_dir));
    }

    // A Joy project must have a founding member: the root of the attestation
    // chain. `joy project member add` later attests new members with the
    // CALLER's key, so without a founder there is no way to bootstrap one. Resolve
    // the founder's identity (--user or git user.email) BEFORE writing anything,
    // and fail fast with guidance rather than leave a member-less project on disk
    // that cannot be recovered without re-init (JOY-01CA-AF).
    let founder_email = match resolve_founder_email(root, options.user.as_deref())? {
        Some(email) => email,
        // Nothing on file and nothing given. A host with a person in front
        // of it asks; every other host refuses by name (D3.9) instead of
        // telling a server to run `git config`.
        None => ask_for_founder_address(root, options.host, options.ask.as_deref_mut())?,
    };

    // Detect or initialize git
    let vcs = default_vcs();
    let git_existed = vcs.is_repo(root);
    let mut git_initialized = false;
    if !git_existed {
        vcs.init_repo(root)?;
        git_initialized = true;
    }

    // Create directory structure. The legacy `.joy/ai/` stores are
    // deliberately absent: jobs are items in `.joy/jobs/` (JOY-01FE-37)
    // and AI members carry their execution config on project.yaml, not in
    // `.joy/ai/agents/`. The 2026-07 repo migrations remove any leftover
    // `.joy/ai/jobs/` and `.joy/ai/agents/`; creating them here would flag
    // every fresh project as pending those migrations.
    let dirs = [
        store::ITEMS_DIR,
        store::MILESTONES_DIR,
        store::RELEASES_DIR,
        store::LOG_DIR,
    ];
    for dir in &dirs {
        let path = joy_dir.join(dir);
        std::fs::create_dir_all(&path).map_err(|e| JoyError::CreateDir {
            path: path.clone(),
            source: e,
        })?;
    }

    // Derive project name and acronym
    let name = options.name.unwrap_or_else(|| {
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string()
    });
    let acronym = options.acronym.unwrap_or_else(|| derive_acronym(&name));

    // Write config and project defaults (embedded files)
    embedded::sync_files(root, CONFIG_FILES)?;
    embedded::sync_files(root, PROJECT_FILES)?;

    let mut project = Project::new(name, Some(acronym));
    if let Some(lang) = options.language.filter(|s| !s.is_empty()) {
        project.language = lang;
    }

    // Register the founder resolved above with all capabilities. A fresh project
    // is always `open`, so they are keyed by e-mail here; `joy init --anonymous`
    // migrates afterwards via switch_to_anonymous.
    project.register_member(
        &founder_email,
        crate::model::project::Member::new(crate::model::project::MemberCapabilities::All),
    )?;

    store::write_yaml(&joy_dir.join(store::PROJECT_FILE), &project)?;

    // This device founded the project, so this device acts as the founder
    // until somebody says otherwise (D3.9). Without the pin, the next
    // command on a machine with no git config would have to guess, and a
    // guess read from the committed project file would let anyone who
    // clones the project claim the founder's member.
    crate::identity::pin_acting_member(root, &project, &founder_email);

    let project_rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    let defaults_rel = format!("{}/{}", store::JOY_DIR, store::CONFIG_DEFAULTS_FILE);
    crate::git_ops::auto_git_add(root, &[&project_rel, &defaults_rel]);

    // Ensure .joy/credentials.yaml is in .gitignore
    ensure_gitignore(root)?;

    // Register the YAML / log merge driver in .gitattributes and git config.
    ensure_gitattributes(root)?;

    // The forge merges with plain git and runs no driver, so the CI file
    // that does the merge where joy is installed goes in right away
    // (JOY-02AC-53). Without a remote there is no forge to write for
    // yet: `joy init ci --forge <name>` adds it later.
    if let Some(template) = ci_template_for_remote(root) {
        if let Err(e) = write_ci_template(root, &template) {
            tracing::debug!(error = %e, "CI merge file not written");
        }
    }
    register_merge_driver(root)?;

    // Install hooks
    install_hooks(root)?;

    // Stamp the per-clone version marker so the first joy invocation
    // after init does not re-trigger the auto-sync routine. The CLI
    // caller is responsible for the cargo-pkg version string; we use
    // the joy-core version here as a stable proxy.
    let _ = set_last_sync_version(root, env!("CARGO_PKG_VERSION"));

    Ok(InitResult {
        project_dir: joy_dir,
        git_initialized,
        git_existed,
        founder: founder_email,
    })
}

/// The founding address from the person at this terminal, or the named
/// refusal of D3.9. An `Interactive` host without an ask (a `--json` run,
/// a piped stdin) refuses like a background host: there is nobody to answer.
///
/// A typed address passes the same alias guard as `--user`, and a refused
/// one is ASKED AGAIN rather than ending the run: the person who pastes
/// the noreply address their forge shows them has made the same kind of
/// mistake as a typo, and the ask already exists to let them correct one.
/// After [`FOUNDER_ASK_TRIES`] answers joy gives up with the same named
/// refusal a host that cannot ask gets.
fn ask_for_founder_address(
    root: &Path,
    host: HostKind,
    ask: Option<&mut (dyn AskFounderAddress + 'static)>,
) -> Result<String, JoyError> {
    if !host.may_ask() {
        return Err(JoyError::NoFounderIdentity);
    }
    let Some(ask) = ask else {
        return Err(JoyError::NoFounderIdentity);
    };
    for _ in 0..FOUNDER_ASK_TRIES {
        let answer = ask
            .ask_founder_address()?
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .ok_or(JoyError::NoFounderIdentity)?;
        match refuse_forge_alias(root, &answer) {
            Ok(()) => return Ok(answer),
            Err(alias @ JoyError::FounderAliasIdentity(_)) => {
                ask.reject_founder_address(&alias.to_string())?;
            }
            Err(e) => return Err(e),
        }
    }
    Err(JoyError::NoFounderIdentity)
}

/// Resolve the founding member's e-mail: an explicit `--user` override, else the
/// git `user.email`. `Ok(None)` when neither is available.
///
/// Capture guard (JOY-0253-8A, epic JOY-0251-AA): a forge ALIAS address
/// must never become a member key — it would split the person's identity
/// (JP-00BF-94). Whether an address is an alias is the responsible forge
/// plugin's judgement alone; without remotes or plugins nothing changes.
fn resolve_founder_email(
    root: &Path,
    user_override: Option<&str>,
) -> Result<Option<String>, JoyError> {
    let email = user_override
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .or_else(|| default_vcs().user_email().ok().filter(|s| !s.is_empty()));
    if let Some(email) = &email {
        refuse_forge_alias(root, email)?;
    }
    Ok(email)
}

/// The capture guard itself, applied to EVERY founding address: the one
/// from git config, the one `--user` names, and the one a person types at
/// the ask. Decided this way on purpose (D4.4): an explicit override is
/// not a reason to let a forge alias become a member key, because the
/// split identity it produces is the same in both cases, and the person
/// who typed it cannot see that their forge handed them an alias. The
/// surfaces that OFFER addresses filter aliases out before they show them,
/// so a person only meets this refusal when they type one themselves.
///
/// Whether an address is an alias stays the responsible plugin's judgement
/// alone; without remotes or plugins nothing changes.
fn refuse_forge_alias(root: &Path, email: &str) -> Result<(), JoyError> {
    let remotes = default_vcs().all_remotes(root).unwrap_or_default();
    let ctx = crate::forge_plugins::CallContext::in_project(root);
    if let Some(spec) = crate::forge_plugins::responsible_plugin(None, &ctx, &remotes) {
        if crate::forge_plugins::resolve(spec, email, &ctx).is_some() {
            return Err(JoyError::FounderAliasIdentity(email.to_string()));
        }
    }
    Ok(())
}

/// Outcome of [`ensure_founder`].
pub enum FounderHeal {
    /// The project already had at least one member; nothing changed.
    AlreadyPresent,
    /// A founder was registered now; carries their e-mail.
    Registered(String),
    /// The project has no members and no identity was available to register one.
    NoIdentity,
}

/// Register a founding member on an already-initialized project that has none.
///
/// Recovers a project that an older Joy `init`ed before a git identity was
/// configured, when the founder step was silently skipped (JOY-01CA-AF). New
/// projects can no longer reach that state because [`init`] now fails fast.
/// Idempotent: does nothing when the project already has members.
///
/// `host` and `ask` are the same two the fresh path takes, and for the
/// same reason (D3.9): this is the command a person runs to repair a
/// project, so a person at a terminal is asked for the address here too
/// instead of being sent to `git config`. A background host answers
/// [`FounderHeal::NoIdentity`] as before.
pub fn ensure_founder(
    root: &Path,
    user_override: Option<&str>,
    host: HostKind,
    ask: Option<&mut (dyn AskFounderAddress + 'static)>,
) -> Result<FounderHeal, JoyError> {
    let project_path = store::joy_dir(root).join(store::PROJECT_FILE);
    let mut project = store::read_project(&project_path)?;
    if project.has_members() {
        return Ok(FounderHeal::AlreadyPresent);
    }
    let email = match resolve_founder_email(root, user_override)? {
        Some(email) => email,
        None => match ask_for_founder_address(root, host, ask) {
            Ok(typed) => typed,
            // The refusal of a host that cannot ask is this function's own
            // "nothing to heal with", which its caller already prints.
            Err(JoyError::NoFounderIdentity) => return Ok(FounderHeal::NoIdentity),
            Err(e) => return Err(e),
        },
    };
    project.register_member(
        &email,
        crate::model::project::Member::new(crate::model::project::MemberCapabilities::All),
    )?;
    store::write_yaml(&project_path, &project)?;
    crate::identity::pin_acting_member(root, &project, &email);
    let rel = format!("{}/{}", store::JOY_DIR, store::PROJECT_FILE);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(FounderHeal::Registered(email))
}

/// Onboard an existing project: set up local environment (hooks, etc.).
pub fn onboard(root: &Path) -> Result<OnboardResult, JoyError> {
    embedded::sync_files(root, CONFIG_FILES)?;
    embedded::sync_files(root, PROJECT_FILES)?;
    ensure_gitignore(root)?;
    ensure_gitattributes(root)?;
    register_merge_driver(root)?;
    let result = install_hooks(root)?;
    // Stamp the per-clone marker so the first joy invocation after
    // onboard does not re-trigger the auto-sync routine.
    let _ = set_last_sync_version(root, env!("CARGO_PKG_VERSION"));
    Ok(result)
}

/// Sync hook files, remember the hook path joy replaces, and set
/// core.hooksPath (design D3.5).
fn install_hooks(root: &Path) -> Result<OnboardResult, JoyError> {
    let actions = embedded::sync_files(root, HOOK_FILES)?;
    let hooks_installed = actions.iter().any(|a| a.action != "up to date");

    // Set core.hooksPath if not already pointing to .joy/hooks
    let vcs = default_vcs();
    let current = vcs.config_get(root, "core.hooksPath").unwrap_or_default();
    let already_set = current == JOY_HOOKS_PATH;

    let mut chained = None;
    if !already_set {
        chained = record_chained_hooks(root, &current)?;
        vcs.config_set(root, "core.hooksPath", JOY_HOOKS_PATH)?;
    }

    Ok(OnboardResult {
        hooks_installed,
        hooks_already_set: already_set,
        chained,
    })
}

/// Remember the hook path joy is about to replace, so every joy hook
/// can run it afterwards (design D3.5), and say so ONCE.
///
/// `core.hooksPath` replaces the hook location entirely, so the
/// alternative - not setting it - means joy's commit-msg check is absent
/// from the person's active hook path and the item rule is enforced for
/// nobody on exactly the team repositories it is written for. Owning the
/// path and chaining keeps husky, lefthook and pre-commit alive beside
/// it.
///
/// Nothing is written when there was no previous value: joy's hooks then
/// chain to git's own `$GIT_DIR/hooks`, which is what git would have run.
/// Nothing is written for joy's own path either, so a second `joy update`
/// cannot overwrite the recorded path with `.joy/hooks` and break the
/// chain.
pub fn record_chained_hooks(root: &Path, previous: &str) -> Result<Option<String>, JoyError> {
    let previous = previous.trim();
    if previous.is_empty() || previous == JOY_HOOKS_PATH {
        return Ok(None);
    }
    let path = store::joy_dir(root).join(CHAINED_PATH_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| JoyError::CreateDir {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::write(&path, format!("{previous}\n")).map_err(|e| JoyError::WriteFile {
        path: path.clone(),
        source: e,
    })?;
    // Said once, here, because this is the one moment the takeover
    // happens: afterwards `core.hooksPath` is joy's and nothing records
    // anything again. stderr, so a `--json` answer stays one envelope.
    eprintln!("joy installed its hooks and kept yours: {previous} still runs after joy's.");
    Ok(Some(previous.to_string()))
}

pub const GITIGNORE_BLOCK_START: &str = "### joy:start -- managed by joy, do not edit manually";
pub const GITIGNORE_BLOCK_END: &str = "### joy:end";

pub const GITIGNORE_BASE_ENTRIES: &[(&str, &str)] = &[
    (".joy/config.yaml", "personal config"),
    (
        ".joy/chat-state.yaml",
        "personal chat state (pins, order, last read)",
    ),
    (".joy/credentials.yaml", "secrets"),
    (".joy/hooks/", "git hooks"),
    (".joy/project.defaults.yaml", "embedded project defaults"),
];

/// Update the joy-managed block in .gitignore with the given entries.
/// Each entry is (path, comment). Replaces the block if it exists, appends otherwise.
pub fn update_gitignore_block(root: &Path, entries: &[(&str, &str)]) -> Result<(), JoyError> {
    let gitignore_path = root.join(".gitignore");

    let mut lines = String::new();
    for (path, _comment) in entries {
        lines.push_str(path);
        lines.push('\n');
    }
    let block = format!(
        "{}\n{}{}",
        GITIGNORE_BLOCK_START, lines, GITIGNORE_BLOCK_END
    );

    let content = if gitignore_path.is_file() {
        let existing =
            std::fs::read_to_string(&gitignore_path).map_err(|e| JoyError::ReadFile {
                path: gitignore_path.clone(),
                source: e,
            })?;
        if existing.contains(GITIGNORE_BLOCK_START) && existing.contains(GITIGNORE_BLOCK_END) {
            let start = existing.find(GITIGNORE_BLOCK_START).unwrap();
            let end = existing.find(GITIGNORE_BLOCK_END).unwrap() + GITIGNORE_BLOCK_END.len();
            let mut updated = String::new();
            updated.push_str(&existing[..start]);
            updated.push_str(&block);
            updated.push_str(&existing[end..]);
            updated
        } else {
            let trimmed = existing.trim_end();
            if trimmed.is_empty() {
                format!("{}\n", block)
            } else {
                format!("{}\n\n{}\n", trimmed, block)
            }
        }
    } else {
        format!("{}\n", block)
    };

    // Idempotency: skip write + auto-stage when content already matches.
    if gitignore_path.is_file() {
        if let Ok(existing) = std::fs::read_to_string(&gitignore_path) {
            if existing == content {
                return Ok(());
            }
        }
    }

    std::fs::write(&gitignore_path, &content).map_err(|e| JoyError::WriteFile {
        path: gitignore_path,
        source: e,
    })?;
    crate::git_ops::auto_git_add(root, &[".gitignore"]);
    Ok(())
}

fn ensure_gitignore(root: &Path) -> Result<(), JoyError> {
    update_gitignore_block(root, GITIGNORE_BASE_ENTRIES)
}

pub const GITATTRIBUTES_BLOCK_START: &str = "### joy:start -- managed by joy, do not edit manually";
pub const GITATTRIBUTES_BLOCK_END: &str = "### joy:end";

/// Path-pattern -> Git attribute lines for the joy-managed
/// `.gitattributes` block. The YAML driver covers every Joy YAML file
/// (items, milestones, releases, project metadata). The log entry uses
/// Git's built-in union driver as an interim until JOY-0112 (Merkle-DAG
/// log) ships.
pub const GITATTRIBUTES_BASE_ENTRIES: &[&str] = &[
    ".joy/items/*.yaml merge=joy-yaml",
    ".joy/milestones/*.yaml merge=joy-yaml",
    ".joy/releases/*.yaml merge=joy-yaml",
    ".joy/ai/agents/*.yaml merge=joy-yaml",
    ".joy/chats/*.yaml merge=joy-yaml",
    ".joy/ai/jobs/*.yaml merge=joy-yaml",
    ".joy/project.yaml merge=joy-yaml",
    ".joy/config.defaults.yaml merge=joy-yaml",
    ".joy/logs/*.log merge=union",
];

/// The CI file that makes a Joy project mergeable from the forge's web
/// interface (JOY-02AC-53), one per forge. A forge merges with plain
/// git, which runs no merge driver, so the merge happens in CI where joy
/// is installed and the button only fast forwards afterwards.
pub struct CiTemplate {
    /// Where the file belongs, relative to the repository root.
    pub target: &'static str,
    pub content: &'static str,
    /// What the person still has to do, if anything.
    pub note: Option<&'static str>,
}

pub const CI_TEMPLATE_MARKER: &str = "# joy:start -- managed by joy";

pub const CI_GITHUB: CiTemplate = CiTemplate {
    target: ".github/workflows/joy-merge.yml",
    content: include_str!("../data/ci/github.yml"),
    note: None,
};

pub const CI_GITEA: CiTemplate = CiTemplate {
    target: ".gitea/workflows/joy-merge.yml",
    content: include_str!("../data/ci/gitea.yml"),
    note: None,
};

pub const CI_GITLAB: CiTemplate = CiTemplate {
    target: ".joy/ci/gitlab.yml",
    content: include_str!("../data/ci/gitlab.yml"),
    note: Some(
        "add to .gitlab-ci.yml:\n  include:\n    - local: .joy/ci/gitlab.yml\nand set the CI variable JOY_PUSH_TOKEN",
    ),
};

/// The template for a forge, by the name a person types or a plugin id.
pub fn ci_template_for(forge: &str) -> Option<CiTemplate> {
    match forge.trim().to_ascii_lowercase().as_str() {
        "github" | "github-enterprise" | "ghes" => Some(CI_GITHUB),
        "gitea" | "forgejo" | "codeberg" => Some(CI_GITEA),
        "gitlab" => Some(CI_GITLAB),
        _ => None,
    }
}

/// The template for the forge this checkout pushes to, read from the
/// remote. `None` when there is no remote yet, which is the local
/// repository a person adds the file to later with `joy init ci`.
pub fn ci_template_for_remote(root: &Path) -> Option<CiTemplate> {
    let vcs = default_vcs();
    let remote = vcs.default_remote(root).ok()?;
    let url = vcs.remote_url(root, &remote).ok()?;
    let host = crate::vcs::remote_url::RemoteUrl::parse(&url)
        .map(|parsed| parsed.host)
        .unwrap_or_default();
    match crate::vcs::forge::known_forge_kind(&host)? {
        crate::vcs::forge::ForgeKind::GitHub | crate::vcs::forge::ForgeKind::GitHubEnterprise => {
            Some(CI_GITHUB)
        }
        crate::vcs::forge::ForgeKind::Gitea => Some(CI_GITEA),
        crate::vcs::forge::ForgeKind::GitLab => Some(CI_GITLAB),
    }
}

/// What writing the CI file did.
#[derive(Debug, PartialEq, Eq)]
pub enum CiWrite {
    Written(String),
    UpToDate(String),
    /// A file of the same name that joy did not write: never touched.
    Foreign(String),
}

/// Write the CI file for `template`, unless a foreign file of that name
/// is in the way. Ours carries the joy marker, so a second run updates
/// it and anything else is left alone.
pub fn write_ci_template(root: &Path, template: &CiTemplate) -> Result<CiWrite, JoyError> {
    let path = root.join(template.target);
    let target = template.target.to_string();
    if path.exists() {
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if !existing.contains(CI_TEMPLATE_MARKER) {
            return Ok(CiWrite::Foreign(target));
        }
        if existing == template.content {
            return Ok(CiWrite::UpToDate(target));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, template.content)?;
    crate::git_ops::auto_git_add(root, &[template.target]);
    Ok(CiWrite::Written(target))
}

pub const MERGE_DRIVER_NAME_KEY: &str = "merge.joy-yaml.name";
pub const MERGE_DRIVER_NAME_VALUE: &str = "Joy YAML merge driver";
pub const MERGE_DRIVER_CMD_KEY: &str = "merge.joy-yaml.driver";
pub const MERGE_DRIVER_CMD_VALUE: &str =
    "joy merge driver --base %O --current %A --other %B --path %P --ours-rev %X --theirs-rev %Y";

/// Update the joy-managed block in .gitattributes with the given lines.
/// Replaces the block if it exists, appends otherwise.
pub fn update_gitattributes_block(root: &Path, lines: &[&str]) -> Result<(), JoyError> {
    let path = root.join(".gitattributes");

    let mut joined = String::new();
    for line in lines {
        joined.push_str(line);
        joined.push('\n');
    }
    let block = format!(
        "{}\n{}{}",
        GITATTRIBUTES_BLOCK_START, joined, GITATTRIBUTES_BLOCK_END
    );

    let content = if path.is_file() {
        let existing = std::fs::read_to_string(&path).map_err(|e| JoyError::ReadFile {
            path: path.clone(),
            source: e,
        })?;
        if existing.contains(GITATTRIBUTES_BLOCK_START)
            && existing.contains(GITATTRIBUTES_BLOCK_END)
        {
            let start = existing.find(GITATTRIBUTES_BLOCK_START).unwrap();
            let end =
                existing.find(GITATTRIBUTES_BLOCK_END).unwrap() + GITATTRIBUTES_BLOCK_END.len();
            let mut updated = String::new();
            updated.push_str(&existing[..start]);
            updated.push_str(&block);
            updated.push_str(&existing[end..]);
            updated
        } else {
            let trimmed = existing.trim_end();
            if trimmed.is_empty() {
                format!("{}\n", block)
            } else {
                format!("{}\n\n{}\n", trimmed, block)
            }
        }
    } else {
        format!("{}\n", block)
    };

    // Idempotency: skip write + auto-stage when content already matches.
    // The lazy-activation hook (called on every joy invocation) relies on
    // this short-circuit to stay cheap and to not dirty the working tree.
    if path.is_file() {
        if let Ok(existing) = std::fs::read_to_string(&path) {
            if existing == content {
                return Ok(());
            }
        }
    }

    std::fs::write(&path, &content).map_err(|e| JoyError::WriteFile { path, source: e })?;
    crate::git_ops::auto_git_add(root, &[".gitattributes"]);
    Ok(())
}

fn ensure_gitattributes(root: &Path) -> Result<(), JoyError> {
    update_gitattributes_block(root, GITATTRIBUTES_BASE_ENTRIES)
}

/// Best-effort registration check, called before every joy invocation
/// that has a project root. Brings `.gitattributes` and the local git
/// merge-driver config in line with the current binary, so users who
/// upgraded joy without re-running `joy init` still get the merge
/// driver. See JOY-0162.
///
/// Idempotent and silent: the file write is skipped when the block is
/// already up to date, and `register_merge_driver` only writes when the
/// stored values differ.
pub fn ensure_lazy_activation(root: &Path) -> Result<(), JoyError> {
    let vcs = default_vcs();
    if !vcs.is_repo(root) {
        return Ok(());
    }
    ensure_gitattributes(root)?;
    register_merge_driver(root)?;
    Ok(())
}

/// Per-clone git config key recording the joy version that last synced
/// this repo. Compared against `env!("CARGO_PKG_VERSION")` to drive the
/// auto-sync hook. See JOY-0164-B5.
pub const LAST_SYNC_VERSION_KEY: &str = "joy.last-sync-version";

/// Read the recorded last-sync version from this clone's git config.
/// `None` if not a repo or the key is unset.
///
/// TODO: route through a `Vcs::config_get` trait method so non-Git
/// backends can implement (ADR-010). Today it's a `GitVcs` inherent
/// method, which constrains the abstraction.
pub fn last_sync_version(root: &Path) -> Option<String> {
    let vcs = default_vcs();
    if !vcs.is_repo(root) {
        return None;
    }
    vcs.config_get(root, LAST_SYNC_VERSION_KEY).ok()
}

/// Stamp the current binary version into this clone's git config.
pub fn set_last_sync_version(root: &Path, version: &str) -> Result<(), JoyError> {
    let vcs = default_vcs();
    if !vcs.is_repo(root) {
        return Ok(());
    }
    vcs.config_set(root, LAST_SYNC_VERSION_KEY, version)
}

/// One-shot core-side sync of a repo against the current binary:
/// `ensure_lazy_activation` + stamp `joy.last-sync-version`. The full
/// `joy update` orchestrator wraps this with the auth and AI refresh
/// routines (see joy-cli's `commands::update::run_full_sync`).
pub fn run_sync(root: &Path, current_version: &str) -> Result<(), JoyError> {
    ensure_lazy_activation(root)?;
    set_last_sync_version(root, current_version)
}

/// Register the joy-yaml merge driver in the local Git config. Idempotent:
/// repeated calls overwrite with the same value. The config is per-clone
/// (Git does not transmit it through clone), so this is also called from
/// `onboard` to bring fresh clones up to date.
fn register_merge_driver(root: &Path) -> Result<(), JoyError> {
    let vcs = default_vcs();
    if !vcs.is_repo(root) {
        return Ok(());
    }
    if vcs.config_get(root, MERGE_DRIVER_NAME_KEY).ok().as_deref() != Some(MERGE_DRIVER_NAME_VALUE)
    {
        vcs.config_set(root, MERGE_DRIVER_NAME_KEY, MERGE_DRIVER_NAME_VALUE)?;
    }
    if vcs.config_get(root, MERGE_DRIVER_CMD_KEY).ok().as_deref() != Some(MERGE_DRIVER_CMD_VALUE) {
        vcs.config_set(root, MERGE_DRIVER_CMD_KEY, MERGE_DRIVER_CMD_VALUE)?;
    }
    Ok(())
}

#[cfg(test)]
mod ci_template_tests {
    use super::*;

    #[test]
    fn the_forge_decides_which_file_is_written() {
        assert_eq!(ci_template_for("github").unwrap().target, CI_GITHUB.target);
        assert_eq!(ci_template_for("Forgejo").unwrap().target, CI_GITEA.target);
        assert_eq!(ci_template_for("gitlab").unwrap().target, CI_GITLAB.target);
        assert!(ci_template_for("sourcehut").is_none());
    }

    #[test]
    fn every_template_calls_joy_and_carries_the_marker() {
        for template in [CI_GITHUB, CI_GITEA, CI_GITLAB] {
            assert!(
                template.content.starts_with(CI_TEMPLATE_MARKER),
                "{} misses the marker",
                template.target
            );
            assert!(
                template.content.contains("joy merge ci"),
                "{} calls nothing",
                template.target
            );
        }
    }

    #[test]
    fn a_file_of_someone_else_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("joy-ci-tpl-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        let path = dir.join(CI_GITHUB.target);

        // ours goes in, and a second run finds it up to date
        assert_eq!(
            write_ci_template(&dir, &CI_GITHUB).unwrap(),
            CiWrite::Written(CI_GITHUB.target.to_string())
        );
        assert_eq!(
            write_ci_template(&dir, &CI_GITHUB).unwrap(),
            CiWrite::UpToDate(CI_GITHUB.target.to_string())
        );

        // a workflow of the person's own with that name stays as it is
        std::fs::write(&path, "name: mine\n").unwrap();
        assert_eq!(
            write_ci_template(&dir, &CI_GITHUB).unwrap(),
            CiWrite::Foreign(CI_GITHUB.target.to_string())
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "name: mine\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// An ask with the answers a person would have typed. `None` is the
    /// person who typed nothing.
    struct ScriptedAsk {
        answers: Vec<Option<String>>,
        asked: usize,
    }

    impl ScriptedAsk {
        fn saying(answer: &str) -> Self {
            ScriptedAsk {
                answers: vec![Some(answer.to_string())],
                asked: 0,
            }
        }

        fn silent() -> Self {
            ScriptedAsk {
                answers: vec![None],
                asked: 0,
            }
        }
    }

    impl AskFounderAddress for ScriptedAsk {
        fn ask_founder_address(&mut self) -> Result<Option<String>, JoyError> {
            let answer = self.answers.get(self.asked).cloned().unwrap_or(None);
            self.asked += 1;
            Ok(answer)
        }
    }

    /// The ask over a fake stdin: one line, and that line is the founder.
    #[test]
    fn the_terminal_ask_reads_the_address_from_its_input() {
        let mut asked = Vec::new();
        let answer = TerminalAsk::new(
            std::io::Cursor::new("founder@example.com\n".as_bytes()),
            &mut asked,
        )
        .ask_founder_address()
        .unwrap();
        assert_eq!(answer.as_deref(), Some("founder@example.com"));
        let shown = String::from_utf8(asked).unwrap();
        assert!(shown.contains("does not know who you are"), "{shown}");
        assert!(shown.contains("Address: "), "{shown}");
    }

    /// A typo is answered with the shape, not with a member named "oops".
    #[test]
    fn the_terminal_ask_asks_again_after_something_that_is_no_address() {
        let mut asked = Vec::new();
        let answer = TerminalAsk::new(
            std::io::Cursor::new("oops\nfounder@example.com\n".as_bytes()),
            &mut asked,
        )
        .ask_founder_address()
        .unwrap();
        assert_eq!(answer.as_deref(), Some("founder@example.com"));
        assert!(String::from_utf8(asked)
            .unwrap()
            .contains("looks like you@example.com"));
    }

    /// Three typos end the ask instead of looping forever, and a closed
    /// stdin ends it at once: both are "nobody answered".
    #[test]
    fn the_terminal_ask_gives_up_instead_of_looping() {
        let mut asked = Vec::new();
        assert_eq!(
            TerminalAsk::new(std::io::Cursor::new("a\nb\nc\nd\n".as_bytes()), &mut asked)
                .ask_founder_address()
                .unwrap(),
            None
        );
        let mut closed = Vec::new();
        assert_eq!(
            TerminalAsk::new(std::io::Cursor::new(&b""[..]), &mut closed)
                .ask_founder_address()
                .unwrap(),
            None
        );
        let mut empty = Vec::new();
        assert_eq!(
            TerminalAsk::new(std::io::Cursor::new("\n".as_bytes()), &mut empty)
                .ask_founder_address()
                .unwrap(),
            None
        );
    }

    /// D3.9: a host with a person in front of it takes the typed address.
    #[test]
    fn an_interactive_host_takes_the_address_the_person_types() {
        let dir = tempdir().unwrap();
        let mut ask = ScriptedAsk::saying("founder@example.com");
        let founder =
            ask_for_founder_address(dir.path(), HostKind::Interactive, Some(&mut ask)).unwrap();
        assert_eq!(founder, "founder@example.com");
        assert_eq!(ask.asked, 1);
    }

    /// D3.9: a background or delegated host refuses with the named
    /// sentence and asks nobody, even when an ask is at hand.
    #[test]
    fn a_host_with_nobody_at_it_refuses_by_name() {
        let dir = tempdir().unwrap();
        for host in [HostKind::Background, HostKind::Delegated] {
            let mut ask = ScriptedAsk::saying("founder@example.com");
            let err = ask_for_founder_address(dir.path(), host, Some(&mut ask)).unwrap_err();
            assert_eq!(
                err.to_string(),
                "this project does not know who you are; run joy init --user <address>"
            );
            assert_eq!(ask.asked, 0, "{host:?} must ask nobody");
        }
    }

    /// An interactive host that brought no ask (a `--json` run, a piped
    /// stdin) is in the same position as a background one.
    #[test]
    fn an_interactive_host_without_an_ask_refuses_too() {
        let dir = tempdir().unwrap();
        let err = ask_for_founder_address(dir.path(), HostKind::Interactive, None).unwrap_err();
        assert!(matches!(err, JoyError::NoFounderIdentity));
        let mut silent = ScriptedAsk::silent();
        let err = ask_for_founder_address(dir.path(), HostKind::Interactive, Some(&mut silent))
            .unwrap_err();
        assert!(matches!(err, JoyError::NoFounderIdentity));
    }

    /// The shape check the ask applies: enough to catch a typo, not a
    /// judgement about the address.
    #[test]
    fn an_address_has_one_at_sign_and_no_spaces() {
        assert!(looks_like_an_address("a@b.c"));
        assert!(!looks_like_an_address("a@b@c"));
        assert!(!looks_like_an_address("nobody"));
        assert!(!looks_like_an_address("@example.com"));
        assert!(!looks_like_an_address("me@"));
        assert!(!looks_like_an_address("me @example.com"));
    }

    #[test]
    fn init_creates_directory_structure() {
        let dir = tempdir().unwrap();
        let result = init(InitOptions {
            name: Some("Test Project".into()),
            acronym: Some("TP".into()),
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();

        assert!(result.project_dir.join("items").is_dir());
        assert!(result.project_dir.join("milestones").is_dir());
        // Legacy AI stores are retired: AI members carry their execution
        // config on project.yaml and jobs are items in `.joy/jobs/`. A
        // fresh init must create neither `.joy/ai/agents/` nor
        // `.joy/ai/jobs/` (they would flag the 2026-07 remove-ai-* repo
        // migrations as pending).
        assert!(!result.project_dir.join("ai/agents").exists());
        assert!(!result.project_dir.join("ai/jobs").exists());
        assert!(result.project_dir.join("logs").is_dir());
        assert!(result.project_dir.join("config.defaults.yaml").is_file());
        assert!(result.project_dir.join("project.yaml").is_file());
    }

    #[test]
    fn init_writes_project_metadata() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            name: Some("My App".into()),
            acronym: Some("MA".into()),
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();

        let project: Project =
            store::read_yaml(&store::joy_dir(dir.path()).join(store::PROJECT_FILE)).unwrap();
        assert_eq!(project.name, "My App");
        assert_eq!(project.acronym.as_deref(), Some("MA"));
    }

    #[test]
    fn init_derives_name_from_directory() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            name: None,
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();

        let project: Project =
            store::read_yaml(&store::joy_dir(dir.path()).join(store::PROJECT_FILE)).unwrap();
        // tempdir names vary, just check it's not empty
        assert!(!project.name.is_empty());
        assert!(project.acronym.is_some());
    }

    #[test]
    fn init_fails_if_already_initialized() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            name: Some("Test".into()),
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();

        let err = init(InitOptions {
            name: Some("Test".into()),
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap_err();

        assert!(matches!(err, JoyError::AlreadyInitialized(_)));
    }

    #[test]
    fn init_creates_gitignore_with_credentials_entry() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            name: Some("Test".into()),
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();

        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".joy/credentials.yaml"));
        assert!(content.contains(".joy/config.yaml"));
    }

    #[test]
    fn init_does_not_duplicate_gitignore_block() {
        let dir = tempdir().unwrap();
        // First init creates the block
        init(InitOptions {
            name: Some("Test".into()),
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();
        let first = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        // Re-running ensure_gitignore should not duplicate
        super::ensure_gitignore(dir.path()).unwrap();
        let second = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        assert_eq!(first, second);
        assert_eq!(second.matches(GITIGNORE_BLOCK_START).count(), 1);
    }

    #[test]
    fn init_writes_gitattributes_block_with_joy_yaml_and_union_log() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            name: Some("Test".into()),
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();

        let content = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(content.contains(GITATTRIBUTES_BLOCK_START));
        assert!(content.contains(GITATTRIBUTES_BLOCK_END));
        assert!(content.contains(".joy/items/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/milestones/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/releases/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/ai/agents/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/ai/jobs/*.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/project.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/config.defaults.yaml merge=joy-yaml"));
        assert!(content.contains(".joy/logs/*.log merge=union"));
    }

    #[test]
    fn init_does_not_duplicate_gitattributes_block() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            name: Some("Test".into()),
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();
        let first = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();

        super::ensure_gitattributes(dir.path()).unwrap();
        let second = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();

        assert_eq!(first, second);
        assert_eq!(second.matches(GITATTRIBUTES_BLOCK_START).count(), 1);
    }

    #[test]
    fn init_registers_merge_driver_in_git_config() {
        let dir = tempdir().unwrap();
        init(InitOptions {
            name: Some("Test".into()),
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();

        let vcs = default_vcs();
        let name = vcs.config_get(dir.path(), MERGE_DRIVER_NAME_KEY).unwrap();
        let cmd = vcs.config_get(dir.path(), MERGE_DRIVER_CMD_KEY).unwrap();
        assert_eq!(name, MERGE_DRIVER_NAME_VALUE);
        assert_eq!(cmd, MERGE_DRIVER_CMD_VALUE);
        assert!(cmd.contains("--ours-rev %X"));
        assert!(cmd.contains("--theirs-rev %Y"));
    }

    #[test]
    fn init_initializes_git_if_needed() {
        let dir = tempdir().unwrap();
        let result = init(InitOptions {
            name: Some("Test".into()),
            acronym: None,
            user: Some("test@example.com".to_string()),
            ..InitOptions::new(dir.path().to_path_buf())
        })
        .unwrap();

        assert!(result.git_initialized);
        assert!(!result.git_existed);
        assert!(dir.path().join(".git").is_dir());
    }
}
