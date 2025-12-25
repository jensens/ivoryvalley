# Tributary MCP Server

Use the `tributary` MCP server for all GitHub operations:

- Issues: read, create, edit, comment, assign, label
- Pull/Merge Requests: create, review, merge
- Labels: create, assign, remove
- Workflow: manage issue states and transitions

## Available Tools

All tools are prefixed with `mcp__tributary_`:

- `start_work`, `complete_issue`
- `list_issues`, `get_issue`, `create_issue`, `update_issue`
- `add_comment`, `list_comments`
- `create_pull_request`, `list_pull_requests`
- `add_labels`, `remove_labels`, `list_labels`

## Working with Epics

Epics are managed as platform issues, NOT local files:

1. **Create epic**: Create an issue with `type/epic` label
2. **Create sub-issues**: Create separate issues linked to the epic
3. **Track progress**: Use `list_epics()` to see epics and their sub-issues
4. **Dependencies**: Add "Blocked by #N" to sub-issues that depend on others

**Never create local files like `epics.md`** - all planning happens in platform issues.

## Important

**Do NOT use `gh` CLI directly** - always use the MCP tools instead.
