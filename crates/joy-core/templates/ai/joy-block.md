## Joy Integration

This project uses [Joy](https://github.com/joyint/joy) for product management.

Your Joy member ID: `{{ member_id }}`

End every commit with these trailers:

```
Co-Authored-By: {{ coauthor_line }}
Delegated-By: <operator email from data.delegated_by of your token redemption>
```

Brand names (e.g. `Claude`, `Copilot`) are allowed in the `Co-Authored-By:` trailer above but nowhere else. In commit body prose, code comments, documentation, and Joy item content, refer to yourself by your Joy member ID.

{% if has_skill %}Use the `/joy` skill for backlog work.{% else %}Use Joy CLI commands for backlog work.{% endif %} Never edit files under `.joy/` directly.
