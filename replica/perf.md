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
| mac-1 | (fill in) | Apple Silicon | | macOS | SSD |
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

### Results (reference)
| date | machine | version | metric | median | worst | notes |
|---|---|---|---|---|---|---|
| | | | | | | |

## 2. Pyoxide

### Database layer
Measured with the `db-bench` example (`cargo run --release -p db-service
--example db-bench -- <profile>`) against the Docker databases.

| date | machine | backend | metric | value | notes |
|---|---|---|---|---|---|
| | | | | | |
