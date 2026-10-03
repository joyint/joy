## Joy Integration

This project uses [Joy](https://github.com/joyint/joy) for product management.

Your Joy identity is `data.member` from `joy auth --token <TOKEN> --json`. Never infer it from the AI tool or these instructions. Use `data.session_env` as `--session` on every Joy write. Before redeeming a token, only use read-only Joy commands.

For delegated commits, end the message with:

```
Delegated-By: <operator email from data.delegated_by of your token redemption>
```

The AI tool may add its own `Co-Authored-By:` attribution; Joy does not require one. In commit body prose, code comments, documentation, and Joy item content, refer to yourself by `data.member`.

After authentication, use `joy project member show <data.member>` to read your capabilities and interaction levels. Follow the effective levels and project gates; do not infer them from another tool's settings.

{% if has_skill %}Use the `/joy` skill for backlog work.{% else %}Use Joy CLI commands for backlog work.{% endif %} Never edit files under `.joy/` directly.
