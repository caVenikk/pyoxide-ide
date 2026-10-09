# Performance

Speed is the reason this project exists. Budgets are **initial targets**; they
are validated against a measured baseline of an existing IDE on the same
projects and machines (macOS and Windows) and then enforced in CI.

| Metric | Initial target |
|---|---|
| Cold start to an interactive editor | <= 0.3 s (macOS), <= 0.5 s (Windows) |
| Keystroke to paint, 100k-line file | p99 <= 16 ms |
| Open a 10 MB file | <= 0.2 s, scrolling without dropped frames |
| Idle memory, mid-size project | <= 300 MB |
| Completion popup after the language server answers | <= 100 ms; the UI never waits for the server |

## Measuring
Record baseline and current numbers in `replica/perf.md` (method, machine, OS,
project, tool versions). Regression thresholds are checked in CI. A claim of
"fast" in docs or releases must cite these numbers.

## Rules of thumb
No blocking calls on the UI thread, virtualised lists and trees, incremental
indexing that can be cancelled, GPU rendering, avoid per-keystroke allocations
on hot paths.
