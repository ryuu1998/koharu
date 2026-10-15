# Koharu V2 Git workflow

Git stores checkpoints locally. GitHub backs them up and lets us review a branch
as a pull request (PR). You can ask Codex to perform the steps below in plain language.

## Our starting point

- `upstream` is the original `koharu-rs/koharu` repository. Its push URL is disabled.
- `origin` is your existing `ryuu1998/koharu` fork.
- Local `main` follows upstream and is an untouched reference copy.
- `v2/main` is the clean integration branch for our restored features and optimizations.
- The fork's old `main` and existing PR branches retain the earlier work.
- `archive/previous-*` branches preserve that work, including the unpushed batch optimization.
- `checkpoint/v2-upstream-2026-10-13` marks the exact starting version, `521e20ca`.

This separates the new upstream baseline from the old PR stack. No published
branch needs to be rewritten to start V2.

## One-time setup on another checkout

Open PowerShell in the repository root:

```powershell
.\scripts\setup-git.ps1
```

Git does not activate repository hooks automatically on clone. This script installs
the local branch guards, fast-forward-only pulls, and useful aliases. Configure
your Git name/email if that checkout does not already have them.

## The routine for every change

Start from the latest accepted V2 version with a clean working tree:

```powershell
git status
git switch v2/main
git pull --ff-only origin v2/main
git switch -c feat/restore-drag-drop
```

Use a new branch name for each complete feature. Keep focused commits on that
branch while developing it, then deliver the complete feature in one PR. Keep
unrelated features in separate PRs. Run the smallest relevant debug check and
inspect the patch:

```powershell
git diff
git diff --check
git add -- path/to/changed-file
git diff --cached
git commit -m "feat: restore archive drag-and-drop"
git push -u origin HEAD
gh pr create --repo ryuu1998/koharu --base v2/main --draft --title "Restore archive drag-and-drop" --body-file pr-description.md
```

The file paths and description filename are examples: use the actual reviewed
files. Stage selected files, including deletions, rather than blindly adding every
file in the folder. Keep credentials, models, datasets, and build outputs out of Git.

Review the draft, verify the actual feature, and mark it ready. Use **Squash and
merge** when accepting it on GitHub. One PR then becomes one accepted commit,
making the whole change easy to undo. Codex prepares PRs; merging requires your instruction.

After accepting it:

```powershell
git switch v2/main
git pull --ff-only origin v2/main
```

## Checkpoints and unfinished work

`git commit` is a permanent checkpoint on your feature branch. Make small,
meaningful commits while working. `git graph` shows recent checkpoints, and
`git st` gives a short status.

To temporarily put unfinished changes aside, including untracked files:

```powershell
git stash push -u -m "unfinished export work"
git stash list
```

When ready to resume, use `git stash apply 'stash@{0}'`. Check the result before
dropping the stash. Stashes live only in this checkout and do not include ignored files.

If the wrong file was staged, use `git unstage -- path/to/file`. This preserves
the edited file and just removes it from the next commit.

## Undo a bad merged PR

Find the **squash commit** for that PR on GitHub, or with:

```powershell
gh pr view 4 --repo ryuu1998/koharu --json state,baseRefName,mergeCommit
```

Replace `4` with the PR number. Confirm it is merged into `v2/main`. Then create
an undo branch from the current accepted version:

```powershell
git switch v2/main
git pull --ff-only origin v2/main
git switch -c revert/pr-4
git revert --no-edit COMMIT_SHA
git push -u origin HEAD
gh pr create --repo ryuu1998/koharu --base v2/main --title "Revert PR #4" --body-file revert-description.md
```

Replace `COMMIT_SHA` with that PR's actual squash commit. Revert creates a new
commit that reverses its changes while preserving the history. If the PR used a
merge commit instead, inspect its parents and use `git revert -m 1 COMMIT_SHA`
when the first parent is the integration branch. A rebase-merged PR can contain
several commits: revert its commits from newest to oldest.

Resolve conflicts and use `git revert --continue`, or cancel the operation with
`git revert --abort`. Test the resulting behavior before accepting the undo PR.
To restore an already reverted change later, revert the revert commit in another PR.

An **unmerged** bad PR can simply be closed; it has not changed the accepted branch.
An **unpublished** last commit can be uncommitted with `git reset --soft HEAD~1`;
its file changes remain staged. Do not use this to rewrite a pushed branch.

Avoid `reset --hard`, `clean -fd`, and force pushes as routine undo tools: they can
discard work or rewrite shared history. `git reflog` can find older commit tips
after an accidental local move; rescue one with `git branch rescue/work COMMIT_SHA`.

## Keep up with the original Koharu repository

Update the local reference branch without mixing custom features into it:

```powershell
git fetch upstream
git switch main
git merge --ff-only upstream/main
```

Bring future upstream changes into V2 through their own review branch:

```powershell
git switch v2/main
git pull --ff-only origin v2/main
git switch -c chore/upstream-update
git merge upstream/main
```

Resolve conflicts, run focused checks, and open a PR targeting `v2/main`.
Do not rebase the shared integration branch. Rebase is useful for unpublished
feature branches; the old published PR branches remain as recovery references.

## Offline recovery backup

After saving unfinished work, run:

```powershell
.\scripts\backup-git.ps1
```

This creates and verifies a bundle in `.git-backups`, containing all committed
history and refs. Copy it to another disk for protection against disk failure.
It excludes ignored files and unstashed/uncommitted edits. Restore into a **new** folder:

```powershell
git -c core.autocrlf=false clone -b v2/main "C:\path\to\koharu-backup.bundle" koharu-recovered
git -C koharu-recovered remote set-url origin https://github.com/ryuu1998/koharu.git
git -C koharu-recovered remote add upstream https://github.com/koharu-rs/koharu.git
git -C koharu-recovered remote set-url --push upstream DISABLED
```

Run the setup script in the recovered checkout. The bundles include local checkpoint
tags. Keep those tags local: the inherited release workflow runs on **any pushed
tag**, so `git push --tags` can unintentionally launch releases. GitHub backup
branches preserve the baseline and previous work without pushing those tags.

## Review consolidated features

Batch translation is consolidated on `codex/batch-translation`. Its single
feature commit preserves the working Batch implementation,
supporting import/export changes, development tooling, and the existing Clippy
cleanup. It contains no Library or Reader implementation.

The older implementation-layer PRs are historical references, not an acceptance
sequence. Review the consolidated replacement rather than merging both it and
the old stack. Preserve the old branches until their replacements are verified.
Library and Reader are consolidated on `codex/library-reader`, based on Batch.
Each feature has one commit and its own PR. The Library branch is the complete
development version containing both features; its PR shows only Library changes.

When upstream has advanced beyond the protected `v2/main`, prepare a maintenance
PR containing the upstream commits unchanged. Base Batch on that maintenance
branch and Library on Batch while the maintenance PR is pending. This keeps each
feature diff limited to its own changes without modifying the protected branch.
The current upstream snapshot is `e27a8ccd` on
`codex/upstream-sync-2026-10-15`. No feature PR is merged as part of a rebase.

An upstream-sync PR must preserve upstream ancestry. The current squash-only,
linear-history settings do not provide a merge method that preserves all the
upstream commit identities. Leave that maintenance PR unmerged until a suitable
integration method is explicitly approved; do not squash it or bypass protection.

Before rebasing an unmerged feature, preserve its old tip and parent boundary.
Restack dependent features after their parent, and review the resulting diffs.
After a parent is squash-merged, replay only the dependent feature's own changes
onto the new base; replaying its whole old ancestry can duplicate the parent.
Use `--force-with-lease` only for the feature branches being deliberately restacked,
never for the protected integration branch. Keep upstream synchronization in its
own maintenance PR and do not change repository merge rules without approval.

## Restored features

See [PREVIOUS_CHANGES.md](PREVIOUS_CHANGES.md) for the original commits and
restoration history. Complete features are delivered in separate PRs against the
updated architecture.

References: [GitHub flow](https://docs.github.com/en/get-started/using-github/github-flow)
and [Git revert](https://git-scm.com/docs/git-revert).
