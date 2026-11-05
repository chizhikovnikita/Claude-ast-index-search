#!/bin/sh
set -eu

# Match the destination, not the remote name: direct URL pushes also run here.
destination=$(printf '%s' "${2:-}" | tr '[:upper:]' '[:lower:]')
destination=${destination%/}
destination=${destination%.git}

case "$destination" in
    https://github.com/chizhikovnikita/claude-ast-index-search | \
    git@github.com:chizhikovnikita/claude-ast-index-search | \
    ssh://git@github.com/chizhikovnikita/claude-ast-index-search | \
    ssh://git@github.com:22/chizhikovnikita/claude-ast-index-search | \
    ssh://git@ssh.github.com:443/chizhikovnikita/claude-ast-index-search)
        exit 0
        ;;
esac

# Do not echo the URL: it may contain credentials.
echo 'Push blocked: this project may publish only to chizhikovnikita/Claude-ast-index-search.' >&2
echo 'The defendend repository is read-only, including direct URL pushes.' >&2
exit 1
