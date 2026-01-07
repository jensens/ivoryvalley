# Tributary MCP Server

Use the `tributary` MCP server for all GitHub operations:

- Issues: list, read, create, update
- Workflow: manage issue states and transitions
- Multi-repo: switch between configured repositories

## Available Tools

All tools are prefixed with `mcp__tributary__`:

### Workflow
- `start_work` - Start working on an issue (creates worktree, returns briefing)
- `complete_issue` - Mark issue complete after PR is merged

### Issues
- `list_issues` - List issues by workflow status (ready, in_progress, blocked, epics, all)
- `get_issue_details` - Get full issue details (only if not starting work)
- `create_issue` - Create a new issue (use parent_issue for sub-issues)
- `update_issue` - Update issue title/body
- `add_sub_issue` - Link an existing issue as sub-issue

### Configuration
- `get_workflow_guide` - Get workflow explanation and label names
- `get_project_config` - Get detected project settings and commands
- `list_repositories` - List all configured repositories

### Code Intelligence
- `lsp_diagnostics` - Get errors/warnings for a file
- `lsp_definition` - Go to symbol definition
- `lsp_references` - Find all references to a symbol
- `lsp_hover` - Get type info and documentation
- `lsp_document_symbols` - Get file outline
- `lsp_workspace_symbols` - Search symbols across workspace

**Proactively use LSP tools:**
- Run `lsp_diagnostics` before committing to catch errors early
- Use `lsp_hover` for type information when exploring code
- Use `lsp_references` to find all usages before refactoring
- Prefer LSP tools over grep/search for type-aware code analysis

## Working with Epics

Epics are managed as platform issues, NOT local files:

1. **Create epic**: Create an issue with `type/epic` label
2. **Create sub-issues**: Use `create_issue` with `parent_issue` parameter
3. **Track progress**: Use `list_issues(filter="epics")` to see epics and their sub-issues
4. **Dependencies**: Add "Blocked by #N" to sub-issues that depend on others

**Never create local files like `epics.md`** - all planning happens in platform issues.

## Important

**Do NOT use `gh` CLI directly** - always use the MCP tools instead.
