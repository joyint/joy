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

### Joining an existing project

If you clone a repo that already uses Joy, run `joy init` to set up your local environment:

```sh
git clone <repo-url> && cd <repo>
joy init     # installs commit-msg hook, sets up git hooks path
joy ai setup # optional: configure AI tool integration
```

`joy init` detects the existing project and switches to onboarding mode -- it installs the commit-msg hook and sets `core.hooksPath` without touching project data.

## Features

- **Git-native** -- all data in `.joy/` as YAML, versioned with your code
- **AI-powered** -- estimate, plan, implement, review via pluggable AI tools
- **Single binary** -- one small Rust binary, no runtime dependencies
- **Offline-first** -- no server needed, sync when you want

## Documentation

- [Tutorial](docs/user/Tutorial.md) -- Getting started with an example project
- [Contributing](docs/dev/CONTRIBUTING.md) -- Joy-specific coding conventions

For product vision, architecture, and ADRs see the [umbrella project](https://github.com/joyint/project).

## Status

Early development. Joy is built and managed with itself (dogfooding).

## License

MIT. See [LICENSE](./LICENSE).
