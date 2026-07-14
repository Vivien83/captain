---
id: note_taker
name: Note Taker
version: 1.0.0
description: Take, search, and organize persistent notes in the knowledge graph
timeout_secs: 10
inputs:
  - name: action
    type: string
    required: true
  - name: content
    type: string
    required: false
  - name: tags
    type: string
    required: false
outputs:
  - name: result
    type: string
---

# Note Taker

Persistent note-taking backed by the knowledge graph.

## Actions

- `add <content> [tags]` — save a new note with optional tags
- `search <query>` — find notes by keyword or tag
- `list [tag]` — list all notes or filter by tag
- `delete <note_id>` — remove a note

## Instructions

Notes are stored as entities (type: "note") in the knowledge graph with:
- name: first 50 chars of content
- properties: full content, tags array, created_at timestamp
- facts: linked to tag entities for organization
