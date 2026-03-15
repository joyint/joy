# Joy

**Terminal-native product management that lives in your Git repo.**

Joy is a fast, file-based, AI-powered project management CLI for developers who think product management should live where the code lives. Part of the [Joyint](https://github.com/joyint) ecosystem.

## Quick Start

```sh
cargo install joyint
cd my-project && joy init

joy add story "User login" --priority high
joy add task "OAuth flow" --parent MY-0001
joy start MY-0002
joy close MY-0002
```

## Features

- **Git-native** -- all data in `.joy/` as YAML, versioned with your code
- **AI-powered** -- estimate, plan, implement, review via pluggable AI tools
- **Single binary** -- CLI, TUI, and server in one Rust binary
- **Offline-first** -- no server needed, sync when you want

## Documentation

- [Tutorial](docs/user/Tutorial.md) -- Getting started with an example project
- [Contributing](docs/dev/CONTRIBUTING.md) -- Joy-specific coding conventions

For product vision, architecture, and ADRs see the [umbrella project](https://github.com/joyint/project).

## Status

Early development. Joy is built and managed with itself (dogfooding).

## License

MIT. See [LICENSE](./LICENSE).
