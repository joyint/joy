# ForgeSync -- Joy CLI Perspective

Joy is offline-first and Git-native. Sync is always explicit, never automatic. This document defines how Joy CLI interacts with hosting platforms (forges) and how data flows between local, Joyint, and external forges.

For the Platform (server-side) perspective, see the companion document in the Platform project.

## Forge Configuration

The `forge:` field in `.joy/project.yaml` defines where releases are created and where `joy sync` pushes/pulls:

| Value | Git host | Mirror | Release target |
|-------|----------|--------|----------------|
| `github` | GitHub | -- | GitHub Releases |
| `gitlab` | GitLab | -- | GitLab Releases |
| `gitea` | Gitea/Codeberg | -- | Gitea Releases |
| `joyint` | joyint.com | -- | Joyint Releases |
| `github@joyint` | joyint.com | GitHub | Both (Joyint primary, GitHub mirror) |
| `gitlab@joyint` | joyint.com | GitLab | Both |
| `gitea@joyint` | joyint.com | Gitea | Both |
| `none` | Local only | -- | Git tags only |

The format `<forge>@joyint` means: Git lives on joyint.com, the external forge is a read-only mirror managed by Joyint.

## Forge Setup

Forge setup is a dedicated step, separate from `joy init` and from the first sync or release. The user runs it when they are ready to connect their project to a hosting platform. This avoids surprise wizards during release or sync when the user wants to focus on shipping.

### When does forge setup happen?

- **`joy init` (new project, no remote):** No forge detection. At the end of init, Joy prints: "Using a hosting platform? Run `joy forge setup` to connect." -- analogous to the AI setup hint.
- **`joy init` (clone/onboarding, remote exists):** Same hint at the end. If the user runs `joy forge setup` next, it detects the forge from the remote URL and proposes it.
- **`joy forge setup` (explicit):** The user runs this when they add a remote for the first time or want to change the forge. This is the primary entry point.
- **`joy project set forge <type>` (manual):** Sets the forge without the interactive setup flow. For users who know what they want. No validation at set time -- if prerequisites are missing (e.g. `gh` not installed, Joyint account not set up), `joy sync` and `joy release create --full` detect this and show a clear error with a hint to run `joy forge setup`.

### Forge detection

For external forges, detection is from the git remote URL:

- `github.com` in URL → `github`
- `gitlab.com` or `gitlab` in URL → `gitlab`
- `codeberg.org` or `gitea` in URL → `gitea`
- `joyint.com` in URL → prompt for mirror selection (default: `joyint`)
- No remote or unknown URL → prompt for manual selection or `none`

For Joyint-hosted projects, the remote URL points to `joyint.com` -- the mirror forge cannot be detected from the URL. Instead, `joy forge setup` asks:

```
Detected Joyint. Does this project mirror to an external forge?
  1) No mirror (joyint)
  2) GitHub mirror (github@joyint)
  3) GitLab mirror (gitlab@joyint)
  4) Gitea mirror (gitea@joyint)
Select [1]:
```

### Authentication flow

Selecting a forge triggers the authentication check for that platform. The goal is to complete all setup upfront so that `joy sync` and `joy release` work without interruption later.

**GitHub:**
1. Check if `gh` CLI is installed and authenticated (`gh auth status`)
2. If authenticated: done
3. If `gh` installed but not authenticated: run `gh auth login`
4. If `gh` not installed: print installation instructions, abort setup

**GitLab:**
1. Check if `glab` CLI is available, or prompt for API token
2. Store token securely (future: `.joy/credentials.yaml`)

**Gitea:**
1. Prompt for API token and instance URL
2. Store securely

**Joyint:**
1. Check if already authenticated via `joy auth status` (Platform API)
2. If not: open the Joyint signup/login page in the browser
3. Display a pairing code in the CLI
4. User logs in on the web, enters the pairing code
5. CLI receives API token, stores it in `.joy/credentials.yaml`
6. If the project repo does not exist on Joyint yet: offer to create it

This device-code flow is similar to `gh auth login` and works without exposing tokens in the terminal.

### Changing the forge

Running `joy forge setup` on a project that already has a forge configured shows the current setting and asks if the user wants to change it. Changing the forge re-runs the authentication flow for the new platform.

## Sync Flow

### `forge: github` (direct hosting)

```
Local ←→ GitHub
```

- `joy sync push` = `git push origin`
- `joy sync pull` = `git pull origin`
- Release: `joy release create --full` creates GitHub Release via `gh` CLI

### `forge: joyint` (Joyint only)

```
Local ←→ Joyint
```

- `joy sync push` = `git push origin` (origin = joyint.com)
- `joy sync pull` = `git pull origin`
- Release: `joy release create --full` creates release on joyint.com via Platform API

### `forge: github@joyint` (Joyint + GitHub mirror)

```
Local ←→ Joyint ←→ GitHub (mirror)
```

- `joy sync push` = push to Joyint → Joyint pushes to GitHub mirror
- `joy sync pull` = Joyint fetches from GitHub → Local pulls from Joyint
- Release: created on Joyint, mirrored to GitHub Releases
- Local has one remote (`origin` = joyint.com). GitHub is not a local remote.
- **Divergence:** If Joyint and the GitHub mirror have diverged (e.g. a PR was merged directly on GitHub while new commits were pushed via Joyint), `joy sync pull` reports the divergence and delivers both histories. The user resolves the merge locally, then `joy sync push` updates both Joyint and the mirror.

### `forge: none`

- `joy sync` prints: "No forge configured. Add `forge:` to project.yaml or run `joy init`."
- Releases create local git tags only, no forge release.

## Offline-First

- All Joy data lives in `.joy/` as YAML, committed to Git.
- `joy sync` is the only network operation. Everything else works offline.
- No live sync, no webhooks from CLI side, no background processes.
- The user decides when to sync. Between syncs, the local repo is the source of truth.

## Conflict Resolution

Conflicts are always resolved locally by the developer:

- `joy sync pull` fetches remote changes. If they conflict with local changes, Git reports the conflict.
- The user resolves conflicts locally using standard Git tools.
- Neither Joyint nor the WebUI perform merges or rebases.
- `joy sync push` only succeeds if the local branch is up to date with the remote.

## Tool Requirements

| Forge | Required tool | Check |
|-------|--------------|-------|
| `github` | `gh` (GitHub CLI) | `joy doctor` reports if missing |
| `gitlab` | `glab` (GitLab CLI) or API token | Future |
| `gitea` | API token | Future |
| `joyint` | None (Joy speaks the Platform API natively) | -- |
| `none` | None | -- |

Joy checks tool availability at `joy init` (forge setup) and at `joy release create --full` (before attempting forge release). Missing tools produce clear error messages with installation hints.
