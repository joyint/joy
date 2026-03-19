// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use joy_core::store;

pub fn run() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let root = store::find_project_root(&cwd)
        .ok_or_else(|| anyhow::anyhow!("No Joy project found (run `joy init` first)"))?;

    let config_path = store::joy_dir(&root).join(store::CONFIG_FILE);
    let config: joy_core::model::Config = store::read_yaml(&config_path)?;
    let yaml = serde_yml::to_string(&config)?;
    print!("{yaml}");

    Ok(())
}
