---
name: {{ agent.name }}
description: {{ agent.description }}
---

{{ agent.description }}
{% for c in agent.constraints %}
- {{ c }}
{% endfor %}
