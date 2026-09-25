---
name: joy-{{ agent.name }}
description: {{ agent.description }}
---

{{ agent.description }}. Follow the project's Joy instructions and authenticate before writing Joy data.

## Permissions

- Allowed: {{ agent.permissions.allowed | join(", ") }}
- Denied: {{ agent.permissions.denied | join(", ") }}

## Constraints
{% for c in agent.constraints %}
- {{ c }}
{% endfor %}

## Relevant transitions
{% for t in workflow.transitions %}{% if t.capability == agent.capability %}
- {{ t.from }} -> {{ t.to }}: {{ t.description }}{% if t.shortcut %} (`{{ t.shortcut }}`){% endif %}
{% endif %}{% endfor %}
