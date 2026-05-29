# Fork workflow

This repository is a **fork** of
[defendend/Claude-ast-index-search](https://github.com/defendend/Claude-ast-index-search).
The fork lives at
[chizhikovnikita/Claude-ast-index-search](https://github.com/chizhikovnikita/Claude-ast-index-search).

## Remotes

```
origin    →  https://github.com/chizhikovnikita/Claude-ast-index-search.git  (your fork)
upstream  →  https://github.com/defendend/Claude-ast-index-search.git        (original)
```

Verify with `git remote -v`. If `upstream` is missing:

```bash
git remote add upstream https://github.com/defendend/Claude-ast-index-search.git
```

## Never push to upstream

**Nothing is ever pushed to `upstream` — not commits, not branches, not tags,
not now, not in the future.** `upstream` is fetch-only. Every push goes
exclusively to `origin` (the fork). Syncing means `git fetch upstream` then
merge into `main`; publishing means `git push origin <branch>`.

A git-level guardrail enforces this: `upstream`'s push URL is set to a bogus
value so an accidental push fails to resolve instead of reaching the parent:

```bash
git remote set-url --push upstream DISABLED_no_push_to_upstream
git remote -v   # upstream (push) should read DISABLED_no_push_to_upstream
```

Do not restore or "fix" that push URL.

## Before starting any work — sync with upstream

Stale forks accumulate conflicts. **Always** fetch upstream before writing
code, even for a one-line fix:

```bash
git fetch upstream
git log --oneline origin/main..upstream/main    # see what's new
git checkout main
git merge --ff-only upstream/main               # fast-forward only — fails if main diverged
git push origin main                            # publish to the fork
```

If `--ff-only` fails because `main` has fork-only commits, either rebase
the fork-only commits onto `upstream/main`, or move them to a feature
branch and reset `main` back to `upstream/main`.

For feature branches already in progress off an older base:

```bash
git checkout your-feature-branch
git rebase upstream/main
# resolve conflicts; then:
git push --force-with-lease origin your-feature-branch
```

## Why

The original ships new languages, fixes, and version bumps frequently. A
fork weeks behind is painful to merge — small frequent fetches stay quiet.
Sync on entry, not at PR time.

## Anti-patterns

- **Starting work without `git fetch upstream`.** Code lands on a stale
  base; conflicts surface at PR time.
- **`git pull upstream main`.** Use explicit `fetch` + `merge`/`rebase`;
  `pull` hides which side you're integrating from.
- **Force-pushing to `main` on the fork.** Even on a personal fork. Use
  `--force-with-lease` and only on feature branches.
- **Local commits to the fork's `main`.** Keep `main` mirrored to
  `upstream/main`; do work on feature branches.
- **Pushing to `upstream`.** Never. `upstream` is fetch-only; all pushes go
  to `origin`. The disabled push URL is a guardrail, not an inconvenience.
