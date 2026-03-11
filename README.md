# Joy ✨

**Terminal-native product management that lives in your Git repo.**

Fast, file-based, AI-powered -- for developers who think project management should live where the code lives.

|  | Feature |
|---|---|
| 📁 | **Git-native** -- all data in `.joy/` as YAML, versioned with your code |
| 🤖 | **AI-powered** -- estimate, plan, implement, review via pluggable AI tools |
| ⚡ | **One binary** -- CLI, TUI, and server in a single Rust binary |
| 🌐 | **Cross-platform** -- terminal, browser, desktop, and mobile |
| 🔌 | **Offline-first** -- no server needed, sync when you want |

## Quick Start

```sh
cargo install joyint                                # install
cd my-project && joy init                           # initialize

joy add story "User login" --priority high                  # plan
joy add task "OAuth flow" --parent EP-0001
joy start IT-0001                                   # work
joy submit IT-0001                                  # review

joy ai setup claude-code                            # connect AI
joy ai estimate IT-0002                             # estimate effort
joy ai implement IT-0002 --budget 5.00              # implement
joy ai review IT-0002                               # review result
```

## Status

Early development. Joy is built and managed with itself (dogfooding). 🐕

## Documentation

| Document | Purpose |
|---|---|
| [Tutorial](docs/user/Tutorial.md) | Getting started with an example project |
| [Vision](docs/dev/Vision.md) | Product vision, data model, CLI commands, AI integration |
| [Architecture](docs/dev/Architecture.md) | Tech stack, repo structure, security, ADRs |
| [Contributing](docs/dev/CONTRIBUTING.md) | Coding conventions, testing, CI/CD, commit messages |
| [Backlog](docs/dev/Backlog.md) | Epics, Stories, Tasks in Joy's own format |

## License

We believe great tools should be open. Joy is built to support **joyful development** 🎉

| | What | License |
|---|---|---|
| 🛠️ | CLI, TUI, server, AI dispatch | MIT |
| 🌐 | Web UI | MIT |
| 📱 | Native app (desktop, mobile) | Commercial |
| ☁️ | [joyint.com](https://joyint.com) hosted service | SaaS |

Self-hosting gives you the **full experience** -- same code, same features, no artificial limits. [joyint.com](https://joyint.com) is there for teams who prefer managed hosting, support, and convenience.

See [LICENSE](./LICENSE) and [app/LICENSE](./app/LICENSE) for details.
