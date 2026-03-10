#!/usr/bin/env bash
# Import Backlog.md items into a Joy project.
# Usage: ./examples/import-backlog.sh [joy-binary] [target-dir]
#
# Defaults:
#   joy-binary: ./target/debug/joy
#   target-dir: ./examples/demo-project

set -euo pipefail

JOY="${1:-./target/debug/joy}"
TARGET="${2:-./examples/demo-project}"

if [ ! -f "$JOY" ]; then
    echo "Building joy..."
    cargo build -p joyint
fi

cd "$TARGET"

# Initialize if needed
if [ ! -d ".joy" ]; then
    "$JOY" init --name "Joy" --acronym "JOY"
fi

echo "Importing epics..."

add() {
    "$JOY" add "$@" 2>&1 || echo "  (skipped or failed)"
}

# --- EP-0001: Core CLI ---
add --id EP-0001 -t "Core CLI" -T epic -p critical \
    -m MS-01 \
    -d "Implement the core CLI commands that make Joy usable for daily product management. This is the minimum viable product."

# --- EP-0002: Terminal Output and UX ---
add --id EP-0002 -t "Terminal Output and UX" -T epic -p high \
    -m MS-01 \
    -d "Define and implement the visual language of Joy's terminal output. Colors, indicators, formatting, and user preferences."

# --- EP-0003: AI Integration ---
add --id EP-0003 -t "AI Integration" -T epic -p high \
    -m MS-02 \
    -d "Integrate external AI tools as first-class collaborators. Joy dispatches work to the configured tool and tracks results, costs, and status."

# --- EP-0004: TUI ---
add --id EP-0004 -t "TUI" -T epic -p medium \
    -m MS-02 \
    -d "Ratatui-based terminal UI launched via joy app. Board view, item detail, dependency graph."

# --- EP-0005: Sync and Server ---
add --id EP-0005 -t "Sync and Server" -T epic -p medium \
    -m MS-02 \
    -d "Push/pull synchronization with a central portal. End-to-end encryption for item content."

# --- EP-0006: Web UI and Portal ---
add --id EP-0006 -t "Web UI and Portal" -T epic -p medium \
    -m MS-03 \
    -d "SolidJS web frontend in web/ (MIT). Embedded in joy serve. Board view, item management, roadmap, AI job dispatch."

echo ""
echo "Importing items for EP-0001..."

# IT-0001 is done
add --id IT-0001 -t "joy init: project initialization" -T task -p critical \
    --epic EP-0001 --status closed \
    -d "Create .joy/ directory structure with config.yaml and project.yaml. Detect existing Git repo or initialize one."

add --id IT-0002 -t "joy add: create items interactively" -T story -p critical \
    --epic EP-0001 --deps IT-0001 --status closed \
    -d "Create new items via interactive prompts. Support non-interactive mode via flags for scripting. Generate unique IDs."

add --id IT-0003 -t "joy ls: list and filter items" -T story -p critical \
    --epic EP-0001 --deps IT-0001 --status closed \
    -d "List all active items by default. Support filters: --epic, --type, --status, --priority, --mine, --blocked. Support --tree."

add --id IT-0004 -t "joy status: change item status" -T story -p critical \
    --epic EP-0001 --deps IT-0002 --status closed \
    -d "Change status of an item by ID. Warn on closing epic with open children. Auto-close epics when all children closed."

add --id IT-0005 -t "joy: board overview" -T story -p critical \
    --epic EP-0001 --deps IT-0003 --status closed \
    -d "Running joy without arguments shows a compact project overview. Group items by status. Show counts per status."

add --id IT-0006 -t "joy edit: modify existing items" -T story -p high \
    --epic EP-0001 --deps IT-0002 --status closed \
    -d "Edit item fields interactively or via flags. Update the updated timestamp automatically."

add --id IT-0007 -t "joy rm: delete items" -T story -p high \
    --epic EP-0001 --deps IT-0002 \
    -d "Delete items with confirmation prompt. Support --force and --cascade on epics."

add --id IT-0008 -t "joy show: item detail view" -T story -p high \
    --epic EP-0001 --deps IT-0002 --status closed \
    -d "Display full item details: all fields, dependencies with status, comments, and change history."

add --id IT-0009 -t "joy deps: dependency management" -T story -p high \
    --epic EP-0001 --deps IT-0002 \
    -d "Show dependencies for an item. Add and remove dependencies. Detect and reject circular dependencies."

add --id IT-000A -t "joy milestone: milestone management" -T story -p medium \
    --epic EP-0001 --deps IT-0002 \
    -d "Create, list, show, remove milestones. Link items to milestones. Show progress and risks."

add --id IT-000B -t "joy log: change history" -T story -p medium \
    --epic EP-0001 --deps IT-0001 \
    -d "Chronological log of item changes. Support --since and --item filtering. Derive from git log."

add --id IT-000C -t "joy project: view and edit project metadata" -T task -p medium \
    --epic EP-0001 --deps IT-0001 \
    -d "Display and edit project.yaml fields. Interactive mode when called without flags."

add --id IT-000D -t "joy completions: shell completion generation" -T task -p medium \
    --epic EP-0001 --deps IT-0001 \
    -d "Generate completions for bash, zsh, fish, PowerShell, elvish via clap_complete."

add --id IT-0029 -t "joy assign: assign and unassign items" -T story -p medium \
    --epic EP-0001 --deps IT-0001 \
    -d "Assign an item to a person or agent. Validates email format. Updates assignee field."

add --id IT-002A -t "joy comment: add comments to items" -T story -p medium \
    --epic EP-0001 --deps IT-0001 \
    -d "Add a comment to an item with author and timestamp. Inline text or interactive via EDITOR."

add --id IT-002B -t "Status shortcuts: start, submit, close" -T task -p low \
    --epic EP-0001 --deps IT-0004 \
    -d "Convenience aliases: joy start, joy submit, joy close for common status transitions."

echo ""
echo "Importing items for EP-0002..."

add --id IT-000E -t "Semantic color scheme for terminal output" -T story -p high \
    --epic EP-0002 \
    -d "Implement color scheme using console/owo-colors. Semantic mapping for status, priority, IDs. Respect NO_COLOR."

add --id IT-000F -t "Emoji indicators for item types and status" -T story -p medium \
    --epic EP-0002 \
    -d "Use emoji as visual indicators for type and status. Text fallback. Deactivation via --no-emoji."

add --id IT-0010 -t "Output format configuration in config.yaml" -T task -p medium \
    --epic EP-0002 --deps "IT-000E,IT-000F" \
    -d "Persist output preferences in .joy/config.yaml. CLI flags override config. Env vars override flags."

add --id IT-0011 -t "Compact table formatting for joy ls" -T task -p medium \
    --epic EP-0002 --deps "IT-0003,IT-000E" \
    -d "Aligned columns for ID, title, status, priority. Respect terminal width. Consistent spacing in tree view."

echo ""
echo "Importing items for EP-0003..."

add --id IT-0012 -t "joy ai setup: tool and model configuration" -T story -p high \
    --epic EP-0003 \
    -d "Configure one AI tool per project. Detect installed CLI tools. Write config to .joy/config.yaml."

add --id IT-0013 -t "joy ai estimate: effort and cost estimation" -T story -p high \
    --epic EP-0003 --deps IT-0012 \
    -d "AI analyzes item description, codebase context, and dependencies. Estimate effort and cost."

add --id IT-0014 -t "joy ai plan: break epic into items" -T story -p high \
    --epic EP-0003 --deps IT-0012 \
    -d "Given an epic, AI proposes breakdown into stories, tasks, dependencies. Present for human review."

add --id IT-0015 -t "joy ai implement: dispatch to external agent" -T story -p high \
    --epic EP-0003 --deps IT-0012 \
    -d "Prepare context. Invoke configured AI tool. Track job in .joy/ai/jobs/. Support --budget flag."

add --id IT-0016 -t "joy ai review: automated code review" -T story -p medium \
    --epic EP-0003 --deps IT-0012 \
    -d "AI reviews implementation against acceptance criteria and conventions. Structured pass/fail output."

add --id IT-0017 -t "joy ai status: monitor AI jobs" -T story -p medium \
    --epic EP-0003 --deps IT-0015 \
    -d "Show running AI jobs with progress. Support --history and --costs for aggregation."

add --id IT-0018 -t "AI cost tracking and job logging" -T task -p high \
    --epic EP-0003 --deps IT-0015 \
    -d "Every AI job writes a JOB-ID.yaml with item, type, provider, model, status, tokens, cost, result."

add --id IT-0019 -t "Status intelligence from git activity" -T story -p low \
    --epic EP-0003 --deps "IT-0012,IT-0004" \
    -d "AI analyzes git log and branch names. Suggest status updates based on development activity."

echo ""
echo "Importing items for EP-0004..."

add --id IT-001A -t "joy app: basic board view" -T story -p high \
    --epic EP-0004 \
    -d "Kanban-style board with columns per status. Navigate items with keyboard. Open detail with Enter."

add --id IT-001B -t "TUI: item detail panel" -T story -p medium \
    --epic EP-0004 --deps IT-001A \
    -d "Show full item details in a side panel or overlay. Edit status and priority inline."

add --id IT-001C -t "TUI: dependency tree view" -T story -p low \
    --epic EP-0004 --deps IT-001A \
    -d "Visual dependency graph in the TUI. Highlight blocked items and critical paths."

echo ""
echo "Importing items for EP-0005..."

add --id IT-001D -t "joy serve: HTTP server with REST API" -T story -p high \
    --epic EP-0005 \
    -d "Axum-based server. Support --daemon. REST API for all CRUD operations. Behind server feature flag."

add --id IT-001E -t "joy sync: push and pull" -T story -p high \
    --epic EP-0005 --deps IT-001D \
    -d "Bidirectional sync. Support --push, --pull, --auto. Last-write-wins with conflict detection."

add --id IT-001F -t "joy clone: clone project from remote" -T story -p medium \
    --epic EP-0005 --deps IT-001D \
    -d "Clone a project from a remote server URL. Download all items, milestones, config."

add --id IT-0020 -t "Client-side encryption for synced data (v2)" -T story -p low \
    --epic EP-0005 --deps IT-001E \
    -d "Deferred to v2. AES-256-GCM key per project. Encrypt item content before push."

add --id IT-0021 -t "OAuth authentication with GitHub and Gitea" -T task -p high \
    --epic EP-0005 --deps IT-001D \
    -d "OAuth 2.0 authentication for sync. E-mail as user identity. Server issues JWTs after OAuth login."

echo ""
echo "Importing items for EP-0006..."

add --id IT-0022 -t "Web frontend scaffolding with SolidJS" -T task -p high \
    --epic EP-0006 \
    -d "Set up web/ project with SolidJS + TypeScript + Vite + Tailwind. API client for REST API."

add --id IT-0023 -t "Web: board view (Kanban)" -T story -p high \
    --epic EP-0006 --deps IT-0022 \
    -d "Kanban board with drag-and-drop status changes. Filter by epic, type, priority, assignee."

add --id IT-0024 -t "Web: item detail and editing" -T story -p high \
    --epic EP-0006 --deps IT-0022 \
    -d "View and edit all item fields. Inline editing for status and priority. Comment thread."

add --id IT-0025 -t "Web: visual roadmap and dependency graph" -T story -p medium \
    --epic EP-0006 --deps IT-0023 \
    -d "Timeline view with milestones and epic progress. Interactive dependency graph."

add --id IT-0026 -t "Web: AI job dispatch and monitoring" -T story -p medium \
    --epic EP-0006 --deps "IT-0022,IT-0017" \
    -d "Dispatch AI jobs from the web UI. Monitor progress in real time. Review and approve AI work."

add --id IT-0027 -t "Web: encryption key handling in browser (v2)" -T task -p low \
    --epic EP-0006 --deps "IT-0020,IT-0022" \
    -d "Deferred to v2. Web Crypto API for client-side encryption/decryption. Same as CLI (AES-256-GCM)."

add --id IT-0028 -t "Embed web frontend in joy serve" -T task -p high \
    --epic EP-0006 --deps "IT-001D,IT-0022" \
    -d "Embed built web assets into joy-cli binary. joy serve serves API and web UI on same port."

add --id IT-002C -t "Tauri native shell wrapping web frontend" -T task -p medium \
    --epic EP-0006 --deps IT-0022 \
    -d "Tauri 2 project in app/ wrapping web/ frontend. Desktop and mobile. Offline support."

add --id IT-002D -t "joyint.com deployment" -T task -p medium \
    --epic EP-0006 --deps "IT-001D,IT-0028" \
    -d "Deploy joy serve to joyint.com as managed service. CI/CD pipeline. SSL, monitoring, backups."

echo ""
echo "=== Import complete ==="
echo ""
"$JOY" ls --all
echo ""
"$JOY"
