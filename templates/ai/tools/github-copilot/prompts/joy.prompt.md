# Joy product management assistant

Use this prompt when working with Joy backlog items, milestones, planning, or status tracking.

The `joy` binary is installed and available. Always use Joy CLI commands -- never edit `.joy/` files directly.

## Status changes

- Start work: `joy start <ID>`
- Submit for review: `joy submit <ID>`
- Close: `joy close <ID>`
- Always confirm before changing status

## Workflow
{% for status in workflow.statuses %}
- **{{ status.name }}**: {{ status.description }}{% if status.initial %} (initial){% endif %}{% if status.terminal %} (terminal){% endif %}
{% endfor %}

Transitions:
{% for t in workflow.transitions -%}
- {{ t.from }} -> {{ t.to }}: requires `{{ t.capability }}`{% if t.shortcut %} (`{{ t.shortcut }}`){% endif %}

{% endfor %}
## Implementing items

1. Comment the planned solution: `joy comment <ID> "Plan: ..."`
2. Confirm with the user
3. Run `joy start <ID>` BEFORE writing any code
4. Implement and commit
5. Comment the result with completed todos
6. Run `joy close <ID>` AFTER the implementation is committed
