# ast-index User Guide

This guide is for developers and AI coding agents that need fast, structural
code search inside an existing project.

`ast-index` is a native Rust CLI that builds a local SQLite + FTS5 index of
files, symbols, references, modules, and inheritance. After the first build,
most lookups run in milliseconds instead of repeatedly scanning the whole
repository.

## Quick Start

Install `ast-index`, then build an index from the project root:

```bash
cd /path/to/your/project
ast-index rebuild
ast-index stats
ast-index search "UserRepository"
```

The index is stored outside the repository in your user cache directory. It is
not committed and does not modify source files. To see the exact SQLite path:

```bash
ast-index db-path
```

## Connect It To A Project

For a small or medium project, the default setup is enough:

```bash
cd /path/to/your/project
ast-index rebuild
```

For a large repository, add a project config so `rebuild`, `update`, and
`watch` all scan the same intended paths:

```yaml
# .ast-index.yaml
include:
  - app
  - packages/shared
exclude:
  - vendor
  - generated
```

Then rebuild once:

```bash
ast-index rebuild
```

Use `include` when you only want selected directories from a larger tree. Use
`exclude` for generated or vendored folders that should never enter the index.

## Keeping The Index Fresh

Use three commands for the index lifecycle:

```bash
ast-index rebuild  # full rebuild; use first time or after major tree changes
ast-index update   # incremental update; use after edits, pulls, rebases, checkouts
ast-index watch    # foreground watcher; update automatically on file changes
```

`update` walks the configured project roots, compares files with the database,
indexes new or changed supported source files, and removes deleted files from
the index. It honors `.gitignore`, built-in ignored directories, and
`.ast-index.yaml` `include` / `exclude` settings.

`watch` listens for source-file changes and runs the same incremental update
path after a short debounce. Run it in a long-lived terminal while you work:

```bash
cd /path/to/your/project
ast-index watch
```

Or start it in the background from your current shell:

```bash
ast-index watch &
```

Only one watcher is allowed per project. Stop a foreground watcher with
`Ctrl+C`. For a background watcher, use `jobs`, then `fg` + `Ctrl+C` or
`kill %<job-number>`.

## Git Pulls, Rebases, And Checkouts

After changing branches or pulling new code, run:

```bash
ast-index update
```

If `ast-index watch` is already running, file-system events usually keep the
index fresh automatically. A manual `update` is still a good habit after a large
checkout, rebase, or branch switch because it reconciles the full file list with
the database.

Use `rebuild` instead of `update` when:

- the project root or `.ast-index.yaml` changed significantly;
- generated or vendored folders were added or removed;
- the index looks inconsistent after a very large branch switch;
- you want a clean baseline before sharing results with an agent.

## Git Worktrees

Each git worktree has its own directory, and `ast-index` keys the default cache
by the canonical project-root path. That means each worktree gets its own index.

This is intentional: different worktrees can be on different branches, so
sharing one SQLite index between them would make results stale or incorrect.

Recommended workflow:

```bash
cd /path/to/project-main
ast-index rebuild

cd /path/to/project-feature-worktree
ast-index rebuild
```

After the first rebuild in each worktree, use `ast-index update` or
`ast-index watch` inside that worktree as usual.

## Running From Subdirectories

After an index exists, you can run search commands from subdirectories. Results
are scoped to the current subtree:

```bash
cd /path/to/project
ast-index rebuild
ast-index search "Payment"

cd /path/to/project/services/payments
ast-index search "Payment"  # searches only this subtree
```

In monorepos with nested project markers, read commands may stop at the nearest
nested project. If you intentionally built one parent index and want subprojects
to reuse it, opt in with:

```bash
ast-index --walk-up search "Payment"
# or
AST_INDEX_WALK_UP=1 ast-index search "Payment"
```

## AI Agent Instructions

Build the project index first, then teach your agent when to use it:

```bash
cd /path/to/your/project
ast-index rebuild
```

Add a short rule to your agent instructions:

```text
Use ast-index before grep for code search.
Use `search` for broad lookup, `file` for file names, `symbol` / `class` for
definitions, `usages` for references, `callers` for call sites, and
`implementations` for interfaces or base classes.
Before reading a file longer than 500 lines, call `outline` first and then read
only the relevant range.
If ast-index returns no results, fall back to grep.
```

When spawning subagents, include the same instruction in the subagent prompt.
Many agent systems do not automatically pass project rules to subagents.

## Optional Claude Code Local Automation

If you want Claude Code to ensure an index exists at session start, you can add
a local, uncommitted project setting:

```bash
mkdir -p .claude
printf ".claude/\n" >> .git/info/exclude
```

Example `.claude/settings.json`:

```json
{
  "permissions": {
    "allow": [
      "Bash(ast-index *)"
    ]
  },
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup",
        "hooks": [
          {
            "type": "command",
            "command": "ast-index stats >/dev/null 2>&1 || ast-index rebuild",
            "timeout": 120,
            "statusMessage": "Checking ast-index..."
          }
        ]
      }
    ]
  }
}
```

This is optional. If your team commits agent configuration, adapt the paths and
permissions to your own policy.

## Common Commands

```bash
ast-index search "Payment"              # broad search across files and symbols
ast-index file "PaymentView"            # find files by name
ast-index symbol "PaymentRepository"    # find a symbol
ast-index class "BaseController"        # find class-like definitions
ast-index usages "PaymentRepository"    # find references
ast-index refs "PaymentRepository"      # definitions + imports + usages
ast-index callers "processPayment"      # find call sites
ast-index implementations "Repository"  # find implementations
ast-index hierarchy "BaseController"    # inheritance tree
ast-index outline src/main.rs           # file structure
ast-index imports src/main.rs           # imports/includes
ast-index changed                       # symbols changed in VCS diff
ast-index map                           # compact project map
ast-index conventions                   # detected frameworks and patterns
```

Use JSON for scripts or agents:

```bash
ast-index --format json search "Payment"
```

## Advanced

Run SQL against the SQLite index:

```bash
ast-index query "
  SELECT s.name, s.kind, f.path, s.line
  FROM symbols s
  JOIN files f ON s.file_id = f.id
  WHERE s.name LIKE '%Controller%'
  ORDER BY f.path, s.line
"
```

Use structural search through ast-grep when `sg` is installed:

```bash
ast-index agrep 'if ($COND) { return $RET; }' --lang typescript
```

Add external source roots when a project depends on local sibling code:

```bash
ast-index add-root /path/to/shared-library
ast-index list-roots
ast-index update
```

## Troubleshooting

**Index not found.** Run `ast-index rebuild` from the intended project root.

**Results are stale.** Run `ast-index update`. If the tree changed heavily,
run `ast-index rebuild`.

**The agent cannot find the index.** Check `ast-index stats` manually in the
same directory where the agent runs commands.

**Search from a subproject ignores the parent index.** Use `--walk-up` or
`AST_INDEX_WALK_UP=1` when you intentionally want a parent index to win.

**The index includes too much.** Add `.ast-index.yaml` with `include` and
`exclude`, then run `ast-index rebuild`.
