# Performance report

Baseline numbers of the reference IDE and current Pyoxide numbers, measured
the same way on the same machines. Budgets live in
[docs/performance.md](../docs/performance.md). Record every run in the tables
below: date, machine, OS, tool versions, project, method, numbers.

## 1. Baseline of the reference IDE (to do before editor work)

Measure with a licensed copy of PyCharm, default settings, no extra plugins,
indexes already built unless the metric says otherwise. Repeat each metric
5 times; report the median and the worst run. Use the same machine for
Pyoxide later.

### Machines
| id | machine | CPU | RAM | OS | storage |
|---|---|---|---|---|---|
| mac-1 | laptop | Apple M5, 10 cores | 16 GB | macOS 27.0.1 | SSD |
| win-1 | (fill in) | | | Windows 11 | SSD |

### Test projects
- `small`: a new project with one 200-line file.
- `mid`: a mid-size open-source Python project (for example `requests` or
  `flask` at a fixed tag), about 50k lines.
- `big-file`: one generated `.py` file of 100,000 lines (`scripts`: generate
  with a loop of simple functions so the highlighter has real tokens).
- `huge-file`: a 10 MB text file (`.py` or `.txt`).

### Metrics and how to take them
| metric | how |
|---|---|
| Cold start to interactive editor | Reboot or purge file caches (`sudo purge` on macOS; on Windows log off/on). Start the IDE with `small` as the last project; stop the clock when a keystroke in the open editor appears. Use a screen recording at 60 fps (QuickTime / Xbox Game Bar) and count frames from the launch click to the first painted character. |
| Warm start | Same, second start without purging caches. |
| Keystroke to paint, 100k-line file | Open `big-file`, put the caret in the middle, type 200 characters. Record the screen at 120 fps if possible (or 60 fps); measure frames between key-down (on-screen keyboard overlay or a keystroke visualizer) and the glyph appearing. Report p50/p99. Alternative on Windows: the open-source "Typometer" tool. |
| Open 10 MB file | Open `huge-file` from the project tree; time from double-click to first painted text (screen recording); then scroll with PageDown for 10 s and count dropped frames. |
| Idle memory, mid project | Open `mid`, wait for indexing to finish, wait 2 minutes idle. macOS: Activity Monitor "Memory" of the IDE process tree; Windows: Task Manager "Memory (active private working set)" summed over the process tree. |
| Completion latency | In `mid`, type `requests.` (or an equivalent module attribute) 10 times; time from the dot to the popup (screen recording). |
| Indexing time | Fresh `mid` checkout, invalidate caches, measure until the background-task indicator disappears. |

### Database client (reference)
| metric | how |
|---|---|
| Connect / test connection | Data source to the local Docker database (`docker compose up -d`), click Test Connection; time to the success message. |
| First page | `SELECT * FROM bench_rows` (100,000 rows, see `docs/testing-databases.md`), time from Execute to the first page shown. |
| Fetch all | Same query with the page limit disabled, time to the last row shown. |

### Automated measurement (macOS)
`python3 scripts/perf/pycharm_baseline.py --work <empty dir>` runs the
installed IDE on a temporary copy of the user's configuration (own cache and
log directories, real profile untouched) and reads the IDE's log: "ready" is
the first exit from dumb mode for the project (editor usable), "indexed" the
exit after which no dumb mode follows for 15 s. Times are from JVM start (the
launcher adds about 0.3 s). Test projects: a generated 200-line project and
Flask 3.1.0 (shallow clone), no interpreter configured. Typing latency and the
10 MB file are not covered by the script yet.

### Results (reference)
| date | machine | version | metric | median | worst | notes |
|---|---|---|---|---|---|---|
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | small project, first open, fresh caches: indexed | 8.0 s | - | 1 run; includes attaching bundled shared indexes |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | small project, warm start: ready | 3.46 s | 5.51 s | 5 runs |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | Flask, first open (IDE caches warm): ready / indexed | 1.7 s / 5.6 s | - | 1 run |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | Flask, warm start: ready | 2.60 s | 3.42 s | 3 runs |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | Flask, idle 2 min after indexing: memory | 812 MB RSS (process tree), 643 MB JVM heap used | - | -Xmx2048m |
| | | | keystroke to paint, 10 MB file, completion | | | not measured yet |

## 2. Pyoxide

### Database layer
Measured with the `db-bench` example (`cargo run --release -p db-service
--example db-bench -- <profile>`) against the Docker databases.

Machine `mac-1`: Apple M5 (10 cores), 16 GB, macOS 27.0.1, Rust 1.99.0,
release build; databases in Docker (OrbStack) on the same machine. 100,000
rows of (integer, text, timestamp, decimal), pages of 500, median of 5 runs.

| date | machine | backend | connect | first page | fetch 100k | rows/s | notes |
|---|---|---|---|---|---|---|---|
| 2026-10-09 | mac-1 | postgres 18.6 | 3.6 ms | 3.3 ms | 33.8 ms | 2.96 M | auto-commit streaming cursor; manual mode (SQL cursor) about 75 ms |
| 2026-10-09 | mac-1 | sqlite 3.53.2 | 0.1 ms | 0.2 ms | 28.6 ms | 3.50 M | `:memory:`, generated rows; table scan of a file about 14 ms |
| 2026-10-11 | mac-1 | oracle-thin (Free 23) | 39.1 ms | 2.7 ms | 276.6 ms | 0.36 M | PL/SQL wrapper and REF CURSOR fetch around oracle-rs bugs; 64 KiB packets |
| | | oracle-odpi | | | | | needs Oracle Instant Client |
