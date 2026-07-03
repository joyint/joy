// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Read/write for git-native AI agent configs under `.joy/ai/agents/`
//! (JOY-01EA). One YAML file per AI member, keyed by a file-safe form of
//! the member ref. The API key is a secret referenced out of band and is
//! never stored here.

use std::path::Path;

use crate::error::JoyError;
use crate::member_ref::MemberRef;
use crate::model::agent::Agent;
use crate::store;

fn agents_dir(root: &Path) -> std::path::PathBuf {
    store::joy_dir(root).join(store::AI_AGENTS_DIR)
}

/// File-safe stem for a member ref (e.g. `ai:claude@joy` -> `ai-claude-joy`).
pub fn agent_file_stem(member: &MemberRef) -> String {
    member
        .id()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Save an agent config to `.joy/ai/agents/<member>.yaml` and stage it.
pub fn save_agent(root: &Path, agent: &Agent) -> Result<(), JoyError> {
    let dir = agents_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| JoyError::CreateDir {
        path: dir.clone(),
        source: e,
    })?;
    let filename = format!("{}.yaml", agent_file_stem(&agent.member));
    store::write_yaml(&dir.join(&filename), agent)?;
    let rel = format!("{}/{}/{}", store::JOY_DIR, store::AI_AGENTS_DIR, filename);
    crate::git_ops::auto_git_add(root, &[&rel]);
    Ok(())
}

/// Load the agent config for a member, if present.
pub fn load_agent(root: &Path, member: &MemberRef) -> Result<Option<Agent>, JoyError> {
    let path = agents_dir(root).join(format!("{}.yaml", agent_file_stem(member)));
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(store::read_yaml(&path)?))
}

/// Load every agent config.
pub fn load_agents(root: &Path) -> Result<Vec<Agent>, JoyError> {
    let dir = agents_dir(root);
    let mut agents = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(agents);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        agents.push(store::read_yaml::<Agent>(&path)?);
    }
    agents.sort_by(|a, b| a.member.id().cmp(b.member.id()));
    Ok(agents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::InteractionLevel;
    use crate::model::job::Budget;
    use tempfile::tempdir;

    #[test]
    fn agent_save_load_roundtrip_and_file_stem() {
        let dir = tempdir().unwrap();
        let member = MemberRef::new("ai:claude@joy");
        assert_eq!(agent_file_stem(&member), "ai-claude-joy");

        let mut agent = Agent::new(member.clone(), "mock");
        agent.model = Some("claude-sonnet-4".into());
        agent.provider = Some("anthropic".into());
        agent.default_mode = Some(InteractionLevel::Collaborative);
        agent.budget_default = Some(Budget {
            max_cents: 500,
            currency: "EUR".into(),
        });
        save_agent(dir.path(), &agent).unwrap();

        let loaded = load_agent(dir.path(), &member).unwrap().unwrap();
        assert_eq!(loaded, agent);
        assert_eq!(load_agents(dir.path()).unwrap().len(), 1);
        // the API key is never part of the model
        let yaml =
            std::fs::read_to_string(dir.path().join(".joy/ai/agents/ai-claude-joy.yaml")).unwrap();
        assert!(!yaml.to_lowercase().contains("key"));
    }
}
