---
id: web_search
name: Web Search
version: 1.0.0
description: Search the web using DuckDuckGo Instant Answer API
timeout_secs: 15
inputs:
  - name: query
    type: string
    required: true
outputs:
  - name: results
    type: json
---

# Web Search

Search the web for information using DuckDuckGo.

## Usage

The agent provides a query and receives structured results including abstract, related topics, and source URLs.

```bash
curl -s "https://api.duckduckgo.com/?q=${query}&format=json&no_html=1" | jq '{abstract: .Abstract, source: .AbstractSource, url: .AbstractURL, related: [.RelatedTopics[:5][] | {text: .Text, url: .FirstURL}]}'
```
