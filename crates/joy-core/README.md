# joy-core

Core library for [Joy](https://joyint.com/en/joy) - terminal-native, Git-based product management.

Provides the data model, storage layer, auth, guard system, AI template rendering,
and all business logic that the CLI and other consumers build on.

## Usage

```toml
[dependencies]
joy-core = "0.8"
```

```rust
use joy_core::items;
use joy_core::model::item::ItemType;

let items = items::load_items(&joy_root)?;
let bugs: Vec<_> = items.iter()
    .filter(|i| i.item_type == ItemType::Bug)
    .collect();
```

## License

MIT
