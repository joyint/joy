# joy-cli

CLI for [Joy](https://joyint.com/en/joy) - terminal-native, Git-based product management.

Joy keeps all project data in `.joy/` as YAML files, versioned alongside your code.
No server, no accounts, no runtime dependencies - just one binary.

## Install

```sh
cargo install joy-cli
```

This installs the `joy` binary.

## Quick start

```sh
cd my-project && joy init

joy add story "User login" --priority high
joy add task "OAuth flow" --parent MY-0001
joy start MY-0002
joy close MY-0002
```

## Documentation

See the [Joy website](https://joyint.com/en/joy) and the
[tutorial](https://github.com/joyint/joy/blob/main/docs/user/Tutorial.md).

## License

MIT
