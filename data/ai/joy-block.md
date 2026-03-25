## Joy Integration

This project uses [Joy](https://github.com/joyint/joy) for product management.
Read [.joy/ai/instructions.md](../.joy/ai/instructions.md) for AI collaboration rules.

Your Joy member ID: `{{ member_id }}`

Use your member ID as Co-Authored-By in commits:
`Co-Authored-By: {{ member_id }}`

Do not use AI tool brand names in commits, code comments, or documentation.

{% if has_skill %}Use the `/joy` skill for backlog work.{% else %}Use Joy CLI commands for backlog work.{% endif %} Do not edit `.joy/items/*.yaml` files directly.
