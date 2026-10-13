# Previous changes to restore

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

For each restoration, compare current behavior, port missing behavior, regenerate
derived bridge bindings from their authoritative inputs, and run the smallest
relevant debug checks. Keep export options and their CBZ optimization together
when they share an ownership boundary. Port the batch workspace before its
dependent optimization commit.

The old PR #3 continues to target the old `main`. Create replacement V2 PRs;
leave the old PR and its branch intact until the replacements are accepted.
After the restored features are verified, additional optimizations get their
own measured baseline, representative target-device inputs, correctness
comparison, and separate PR.
