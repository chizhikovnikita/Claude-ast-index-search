# ast-index Agent Plugin

This directory is the shared payload for agent integrations.

## Supported Surfaces

- Claude Code: `.claude-plugin/plugin.json`, `commands/`, and `skills/`.
- Codex: `.codex-plugin/plugin.json` and `skills/`.
- Cursor: `.cursor-plugin/plugin.json`, `skills/`, `rules/`, and
  `commands-cursor/`.

The Claude commands are intentionally separate from the Cursor command because
they write different project configuration files.

## Local Testing

Codex reads the repo marketplace from `.agents/plugins/marketplace.json`.

Cursor can load the plugin locally by symlinking this directory:

```bash
mkdir -p ~/.cursor/plugins/local
ln -s /absolute/path/to/Claude-ast-index-search/plugin ~/.cursor/plugins/local/ast-index
```

Then reload Cursor and verify the `ast-index` skill, rule, and
`initialize-ast-index` command appear.
