# 0024. Scoped task checks and fully verified integration batches

- Status: accepted
- Date: 2026-11-12

## Context

A full workspace run per task PR followed by the same checks after each merge
repeats work. Platform-specific defects and interactions between independently
correct changes still need detection before main. A fast check must not be
mistaken for a complete integration check, and a successful result must refer
to the code actually being merged.

## Decision

Develop one task per branch against a protected `integration/*` branch. Task
PRs use the affected-crate dependency closure, conservative platform selection
and relevant database integration. Combine 2-4 reviewed tasks in a batch PR to
main. Every ready main PR and explicit dispatch runs the full three-platform
workspace checks, database integration and existing performance checks.

Select scope against the actual target branch, including renamed/deleted files
and development/build dependencies. Unknown impact or shared build inputs
require full scope. Preserve direct PRs into main with the full gate.

An aggregate required check validates the planned work, executed jobs and live
PR identity. It fails on missing, failed, cancelled or unexpectedly skipped
work. Live draft checks retain the protection against a ready PR inheriting a
stale draft skip. Both protected branch classes require successful checks and
squash PR merges without bypass actors. Main additionally requires an up-to-date
base; integration accepts the checked task heads from the batch's common base.

Freeze the integration base while the batch's task PRs run CI in parallel.
Wait until every included task has passed its required checks and review before
merging the first task; do not enable early per-task auto-merge. Then squash the
checked heads into integration without rebasing each one after its predecessor.
The full main PR validates their combined behavior. If a merge conflicts or the
source changes, resolve it in the affected task branch and run the relevant
checks again. Unplanned base movement while task CI is running invalidates its
recorded base identity and requires a fresh run.

After merging, full checks may be reused only from a successful full PR run
whose immutable evidence identifies the same merged PR, source tree, CI
configuration and toolchain. Evidence is data, never executable content.
Uncertainty defaults to a full run. Reuse also requires a matching successful
full-main cache seed from the preceding six days; new build inputs or an old
seed trigger a full main run to refresh shared dependency caches. This does
not prove the cache entries have not been evicted. Main still builds distributable artifacts
and checks their CLI startup. Caches accelerate work and never prove tests ran.

Before a task push, run scoped local checks and the focused regression or
application scenario. Full local checks are explicit when useful; CI remains
the authoritative combined validation. Release acceptance additionally checks
the packaged application on supported platforms.

## Consequences

Cross-task failures can surface at the batch boundary rather than in each task
PR, but cannot enter main before a complete check. Small batches bound the
investigation and preserve separate task history. Native foundations and other
broad changes may need full checks even in a task branch. Final release testing
does not replace per-task regression checks.

Cache and runner optimizations need measurements. A restored dependency cache
can still intentionally rebuild workspace crates. Keep cache scope and trust
boundaries explicit; never execute cached proof artifacts or silently purchase
additional runner capacity.
