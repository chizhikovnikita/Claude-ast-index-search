# Fork workflow

The working repository is
[chizhikovnikita/Claude-ast-index-search](https://github.com/chizhikovnikita/Claude-ast-index-search).
[defendend/Claude-ast-index-search](https://github.com/defendend/Claude-ast-index-search)
is a read-only source of updates. Never push commits, branches, or tags to
the original repository, including through a direct URL or another remote.
Create local commits on fork branches and publish only to the fork.

## Git protection

Expected remotes:

```text
origin    -> https://github.com/chizhikovnikita/Claude-ast-index-search.git
upstream  -> https://github.com/defendend/Claude-ast-index-search.git (fetch)
upstream  -> DISABLED_no_push_to_upstream (push)
```

Set `remote.pushDefault` to `origin` and retain the disabled upstream push
URL. Install the destination guard with `scripts/install-push-guard.ps1`.
It checks the push URL, so a direct push to the original is also rejected.
The installer preserves an existing pre-push hook and does not change
`core.hooksPath`. Reinstall it in each fresh clone; tracked scripts alone
do not activate Git hooks. Never bypass the guard with `--no-verify`.

Verify URLs before every publication. Do not test the guard by attempting a
push to the original; call the guard with sample destinations instead.
Do not publish to upstream-owned package registries, Homebrew, or Winget.
Release workflow publishing jobs are restricted to the original repository;
fork release artifacts stay in the fork's own GitHub Releases.

## Updating the fork

1. Inspect status, local changes, and fork-only commits. Preserve uncommitted
   changes, local skills, and `.sds`; create a backup ref before integration.
2. Fetch upstream and origin explicitly. Select a stable upstream release
   and record its tag and commit. Fetching is allowed; publishing upstream
   is never allowed.
3. Start a `sync/<release>` branch from the fork's main branch and merge the
   selected upstream revision with `--no-ff`. Resolve conflicts by retaining
   the fork's encoding support and Windows behavior alongside upstream
   functionality. Inspect new file-reading paths for UTF-8-only assumptions.
4. Build and test the release workspace on Windows and Ubuntu. Exercise
   Windows-1251 and mixed-encoding rebuild, update, search, and content output.
   Keep the Windows index publication fix unless upstream already implements
   equivalent behavior. Record failures and compare with the baseline.
5. Commit the reviewed integration on the fork branch. Publish that branch
   explicitly to origin; advance main only after validation. Reinstall the
   CLI from the resulting fork checkout and rebuild its project index.

Main contains fork functionality; it is not a byte-for-byte upstream mirror.
Never reset main to upstream or drop fork-only commits to make a fast-forward
possible. Do not rebase published history, force-push, or move release tags
as part of routine synchronization. Do not use `git pull upstream main`.
The version already supplied by an upstream release does not require running
the publishing `scripts/bump.sh` script.
