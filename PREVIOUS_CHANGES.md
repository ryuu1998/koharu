# Previous changes and V2 restoration

## Current delivery

The Batch restoration is consolidated on `codex/batch-translation`, rebased onto
upstream `e27a8ccd` through `codex/upstream-sync-2026-10-15`. It preserves the
final working Batch feature set at `398e62ad`,
the supporting development and import/export changes, and the existing scene
Clippy cleanup at `36fe4918`, adapted to upstream's telemetry removal. Library
and Reader are excluded from Batch and are maintained separately above it.
The delivery policy and Git instructions now describe
one complete feature per PR.

The older PRs below document how the feature was restored. They are historical
references; do not merge that implementation stack as well as its consolidated
replacement. Keep the original branches and recovery checkpoints available.

## Restoration history

The V2 baseline is original Koharu commit
`521e20ca965eec7fe2498f4ddc25babd4d77f167`, verified against the downloaded source
on October 13, 2026. The source files matched exactly before the workflow additions.

These five feature commits are present in the old local batch branch but absent
from upstream's ancestry. Being absent from the ancestry does **not** prove the
behavior is absent: compare each feature with the current implementation first.

| Order | Work | Original commit | Previous PR |
| --- | --- | --- | --- |
| 1 | Drag-and-drop archive imports | `f7c18b2d31049f970130a42fd109f7aec2be4116` | [#1, merged](https://github.com/ryuu1998/koharu/pull/1) |
| 2 | Project export options | `cc68b5da84d260d6759635882e20fec8f683cb41` | [#2, merged](https://github.com/ryuu1998/koharu/pull/2) |
| 3 | Pipeline CBZ exports | `82882b95193cc6e3d7c962b27cd8befd0ac59d52` | [#2, merged](https://github.com/ryuu1998/koharu/pull/2) |
| 4 | Native batch CBZ translation workspace | `aea48f1165dab0b4d49b90b052b6ea0d52da8b04` | [#3, open on the old main](https://github.com/ryuu1998/koharu/pull/3) |
| 5 | Stabilize and accelerate batch translation | `ee0de0a6cbc11391a1c9ea27fef8ffef8e9201ec` | Saved only locally before this setup |

The previous main is preserved at `archive/previous-main`; the previous export
branch at `archive/previous-export`; the drag-and-drop branch at
`archive/previous-drag-drop`; and the complete batch stack, including the newer
optimization, at `archive/previous-batch`. These refs make the work recoverable
without depending on the older folders remaining on disk.

Inspect a previous patch with `git show COMMIT_SHA`. Reapply only its feature
changes on a new branch from `v2/main`; do not copy the whole old source tree or
replay the old upstream merge commits. Cherry-pick can carry a patch forward,
but conflicts must be adapted to the current architecture. Use
`git cherry-pick --abort` to cancel an unsuccessful replay.

The published features have been adapted to the current architecture in separate
draft PRs. Generated bridge bindings were regenerated from Rust, and focused debug
checks cover the affected behavior. The latest local branch includes every step.

| V2 PR | Scope |
| --- | --- |
| [#5](https://github.com/ryuu1998/koharu/pull/5) | Saved debug launcher and frontend hot reload |
| [#6](https://github.com/ryuu1998/koharu/pull/6) | Native drag-and-drop imports |
| [#7](https://github.com/ryuu1998/koharu/pull/7) | Current-page and project export choices |
| [#8](https://github.com/ryuu1998/koharu/pull/8) | Compact CBZ and bounded project exports |
| [#9](https://github.com/ryuu1998/koharu/pull/9) | Batch folder discovery and chapter covers |
| [#10](https://github.com/ryuu1998/koharu/pull/10) | Isolated native batch processing and cancellation |
| [#11](https://github.com/ryuu1998/koharu/pull/11) | Headless batch CLI |
| [#12](https://github.com/ryuu1998/koharu/pull/12) | Batch browser redesign and workspace navigation |
| [#13](https://github.com/ryuu1998/koharu/pull/13) | Batch run/stop controls, chapter status, and progress |

PRs remain unmerged. Follow the consolidated-feature instructions in
[GIT_WORKFLOW.md](GIT_WORKFLOW.md). Existing upstream archive decoding, settings,
and model lifecycle are reused rather than copying the older implementations.

The unpublished optimization commit `ee0de0a6` remains saved on
`archive/previous-batch`. Its additional retry/concurrency and model changes need
separate review and device measurements; they are not part of this restoration.

The old PR #3 continues to target the old `main`. Create replacement V2 PRs;
leave the old PR and its branch intact until the replacements are accepted.
After the restored features are verified, additional optimizations get their
own measured baseline, representative target-device inputs, correctness
comparison, and separate PR.
