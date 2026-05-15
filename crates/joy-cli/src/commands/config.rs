// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use std::fs;

use joy_core::store;

use crate::color;

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
    /// Dotted key path (e.g. output.emoji, agents.architect.mode). A
    /// trailing `.*` lists every leaf under that prefix.
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_config_key))]
    key: String,

    /// Append a one-line semantic description to each value. Useful for
    /// AI tools that need to know what each setting actually does.
    #[arg(long)]
    describe: bool,
}

#[derive(clap::Args)]
struct SetArgs {
    /// Dotted key path
    #[arg(add = clap_complete::engine::ArgValueCompleter::new(crate::complete::complete_config_key))]
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
        Some(ConfigCommand::Get(a)) => get_value(&a.key, a.describe),
        Some(ConfigCommand::Set(a)) => set_value(&root, &a.key, &a.value),
    }
}

fn show_all() -> anyhow::Result<()> {
    let value = store::load_config_value();
    let personal = store::load_personal_config_value();

    if crate::output::is_json() {
        #[derive(serde::Serialize)]
        struct ConfigPayload {
            effective: serde_json::Value,
            personal: serde_json::Value,
        }
        return crate::output::emit(ConfigPayload {
            effective: value,
            personal,
        });
    }

    println!("{}", color::header("Configuration"));

    let obj = value.as_object().cloned().unwrap_or_default();
    let sections: Vec<_> = obj.iter().collect();

    for (i, (key, val)) in sections.iter().enumerate() {
        if i > 0 {
            println!();
        }
        if val.is_object() {
            println!("{}", color::section(key));
            print_object(val, &personal, &[key.as_str()], 2);
        } else {
            let is_default = !has_key(&personal, &[key.as_str()]);
            println!("{}", format_kv(key, val, 16, is_default));
        }
    }

    println!("{}", color::label(&"-".repeat(color::terminal_width())));
    Ok(())
}

fn print_object(
    val: &serde_json::Value,
    personal: &serde_json::Value,
    path: &[&str],
    indent: usize,
) {
    let Some(obj) = val.as_object() else {
        return;
    };
    let pad = " ".repeat(indent);
    for (k, v) in obj {
        if v.is_object() {
            println!("{}{}", pad, color::label(k));
            let mut next_path = path.to_vec();
            next_path.push(k.as_str());
            print_object(v, personal, &next_path, indent + 2);
        } else {
            let mut key_path = path.to_vec();
            key_path.push(k.as_str());
            let is_default = !has_key(personal, &key_path);
            println!("{}{}", pad, format_kv(k, v, 14, is_default));
        }
    }
}

fn format_kv(key: &str, val: &serde_json::Value, width: usize, is_default: bool) -> String {
    let formatted = format_value(val, is_default);
    let suffix = if is_default {
        format!(" {}", color::inactive("[default]"))
    } else {
        String::new()
    };
    format!(
        "{:<w$} {}{}",
        color::label(key),
        formatted,
        suffix,
        w = width
    )
}

fn format_value(val: &serde_json::Value, is_default: bool) -> String {
    let raw = match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => return color::inactive("null"),
        other => other.to_string(),
    };
    if is_default {
        color::success(&raw)
    } else {
        raw
    }
}

fn has_key(value: &serde_json::Value, path: &[&str]) -> bool {
    let mut current = value;
    for part in path {
        match current.get(*part) {
            Some(v) => current = v,
            None => return false,
        }
    }
    true
}

fn get_value(key: &str, describe: bool) -> anyhow::Result<()> {
    let value = store::load_config_value();

    // Wildcard form: `modes.*` lists every leaf under that prefix.
    if let Some(prefix) = wildcard_prefix(key) {
        return get_wildcard(&value, key, prefix, describe);
    }

    let result = navigate(&value, key);

    if crate::output::is_json() {
        #[derive(serde::Serialize)]
        struct GetPayload<'a> {
            key: &'a str,
            value: serde_json::Value,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
        }
        let description = if describe {
            result.and_then(|v| joy_core::model::config::describe_value(key, v))
        } else {
            None
        };
        return crate::output::emit(GetPayload {
            key,
            value: result.cloned().unwrap_or(serde_json::Value::Null),
            description,
        });
    }

    let Some(result) = result else {
        // Exit silently with code 1 -- callers check the exit code
        std::process::exit(1);
    };

    let suffix = if describe {
        joy_core::model::config::describe_value(key, result)
            .map(|d| format!("  {} {}", color::inactive("-"), color::inactive(&d)))
            .unwrap_or_default()
    } else {
        String::new()
    };

    match result {
        serde_json::Value::String(s) => println!("{s}{suffix}"),
        serde_json::Value::Bool(b) => println!("{b}{suffix}"),
        serde_json::Value::Number(n) => println!("{n}{suffix}"),
        serde_json::Value::Null => println!("null{suffix}"),
        other => {
            // Objects do not have a single description; render as YAML.
            let yaml = serde_yaml_ng::to_string(&other)?;
            print!("{yaml}");
        }
    }

    Ok(())
}

/// Strip a trailing `.*` (or bare `*`) and return the prefix. None if
/// the key has no wildcard.
fn wildcard_prefix(key: &str) -> Option<&str> {
    if key == "*" {
        Some("")
    } else if let Some(rest) = key.strip_suffix(".*") {
        Some(rest)
    } else {
        None
    }
}

fn get_wildcard(
    config: &serde_json::Value,
    key: &str,
    prefix: &str,
    describe: bool,
) -> anyhow::Result<()> {
    let leaves = joy_core::model::config::flatten_under(config, prefix);

    if crate::output::is_json() {
        #[derive(serde::Serialize)]
        struct Entry {
            key: String,
            value: serde_json::Value,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
        }
        #[derive(serde::Serialize)]
        struct Payload<'a> {
            key: &'a str,
            entries: Vec<Entry>,
        }
        let entries = leaves
            .into_iter()
            .map(|(k, v)| {
                let description = if describe {
                    joy_core::model::config::describe_value(&k, &v)
                } else {
                    None
                };
                Entry {
                    key: k,
                    value: v,
                    description,
                }
            })
            .collect();
        return crate::output::emit(Payload { key, entries });
    }

    if leaves.is_empty() {
        std::process::exit(1);
    }

    let max_key = leaves.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let max_val = leaves
        .iter()
        .map(|(_, v)| scalar_str(v).len())
        .max()
        .unwrap_or(0);

    for (k, v) in &leaves {
        let val_str = scalar_str(v);
        if describe {
            if let Some(desc) = joy_core::model::config::describe_value(k, v) {
                println!(
                    "{:<kw$}  {:<vw$}  {} {}",
                    color::label(k),
                    val_str,
                    color::inactive("-"),
                    color::inactive(&desc),
                    kw = max_key,
                    vw = max_val,
                );
                continue;
            }
        }
        println!("{:<kw$}  {}", color::label(k), val_str, kw = max_key);
    }
    Ok(())
}

fn scalar_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn set_value(root: &std::path::Path, key: &str, raw_value: &str) -> anyhow::Result<()> {
    let local_path = store::local_config_path(root);

    // Read existing local override file, or start with empty object
    let mut value: serde_json::Value = if local_path.is_file() {
        let content = fs::read_to_string(&local_path)?;
        let parsed: serde_json::Value = serde_yaml_ng::from_str(&content)?;
        if parsed.is_null() {
            serde_json::json!({})
        } else {
            parsed
        }
    } else {
        serde_json::json!({})
    };

    let parsed = parse_value(raw_value);
    set_nested(&mut value, key, parsed)?;

    // Validate by round-tripping through YAML (same path as load_config),
    // which correctly handles hyphen/underscore key variants.
    let yaml = serde_yaml_ng::to_string(&value)?;
    let defaults_yaml = serde_yaml_ng::to_string(&joy_core::model::Config::default())?;
    let mut merged: serde_json::Value = serde_yaml_ng::from_str(&defaults_yaml)?;
    let overlay: serde_json::Value = serde_yaml_ng::from_str(&yaml)?;
    store::deep_merge_value(&mut merged, &overlay);
    if serde_json::from_value::<joy_core::model::Config>(merged).is_err() {
        if let Some(hint) = joy_core::model::config::field_hint(key) {
            anyhow::bail!("'{raw_value}' is not valid for '{key}'\n  {hint}");
        }
        anyhow::bail!("'{raw_value}' is not a valid value for '{key}'");
    }

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
