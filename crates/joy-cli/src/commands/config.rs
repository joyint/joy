// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;

use joy_core::store;

#[derive(clap::Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    command: Option<ConfigCommand>,
}

#[derive(clap::Subcommand)]
enum ConfigCommand {
    /// Get a config value by dotted key (e.g. output.fortune)
    Get(GetArgs),
    /// Set a config value by dotted key (e.g. output.fortune false)
    Set(SetArgs),
}

#[derive(clap::Args)]
struct GetArgs {
    /// Dotted key path (e.g. output.emoji, agents.architect.interaction-level)
    key: String,
}

#[derive(clap::Args)]
struct SetArgs {
    /// Dotted key path
    key: String,
    /// Value to set (string, number, or boolean)
    value: String,
}

pub fn run(args: ConfigArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    match args.command {
        None => show_all(),
        Some(ConfigCommand::Get(a)) => get_value(&a.key),
        Some(ConfigCommand::Set(a)) => set_value(&root, &a.key, &a.value),
    }
}

fn show_all() -> anyhow::Result<()> {
    let value = store::load_config_value();
    let yaml = serde_yml::to_string(&value)?;
    print!("{yaml}");
    Ok(())
}

fn get_value(key: &str) -> anyhow::Result<()> {
    let value = store::load_config_value();

    let Some(result) = navigate(&value, key) else {
        // Exit silently with code 1 -- callers check the exit code
        std::process::exit(1);
    };

    match result {
        serde_json::Value::String(s) => println!("{s}"),
        serde_json::Value::Bool(b) => println!("{b}"),
        serde_json::Value::Number(n) => println!("{n}"),
        serde_json::Value::Null => println!("null"),
        other => {
            let yaml = serde_yml::to_string(&other)?;
            print!("{yaml}");
        }
    }

    Ok(())
}

fn set_value(root: &std::path::Path, key: &str, raw_value: &str) -> anyhow::Result<()> {
    let local_path = store::local_config_path(root);

    // Read existing local override file, or start with empty object
    let mut value: serde_json::Value = if local_path.is_file() {
        let content = fs::read_to_string(&local_path)?;
        serde_yml::from_str(&content)?
    } else {
        serde_json::json!({})
    };

    let parsed = parse_value(raw_value);
    set_nested(&mut value, key, parsed)?;

    let yaml = serde_yml::to_string(&value)?;
    fs::write(&local_path, yaml)?;

    println!("{} = {}", key, raw_value);
    Ok(())
}

fn navigate<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for part in key.split('.') {
        // Support both dot-separated and hyphenated keys
        current = current.get(part)?;
    }
    Some(current)
}

fn set_nested(
    value: &mut serde_json::Value,
    key: &str,
    new_val: serde_json::Value,
) -> anyhow::Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    let mut current = value;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last part: set the value
            current
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("cannot set '{}': parent is not an object", key))?
                .insert(part.to_string(), new_val.clone());
            return Ok(());
        }
        // Navigate or create intermediate objects
        if !current.get(*part).is_some_and(|v| v.is_object()) {
            current
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("cannot set '{}': parent is not an object", key))?
                .insert(part.to_string(), serde_json::json!({}));
        }
        current = current.get_mut(*part).unwrap();
    }

    Ok(())
}

fn parse_value(raw: &str) -> serde_json::Value {
    match raw {
        "true" => serde_json::Value::Bool(true),
        "false" => serde_json::Value::Bool(false),
        "null" | "none" => serde_json::Value::Null,
        _ => {
            if let Ok(n) = raw.parse::<i64>() {
                serde_json::Value::Number(n.into())
            } else if let Ok(f) = raw.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or_else(|| serde_json::Value::String(raw.to_string()))
            } else {
                serde_json::Value::String(raw.to_string())
            }
        }
    }
}
