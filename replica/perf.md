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
Flask 3.1.0 (shallow clone), no interpreter configured.

`python3 scripts/perf/pycharm_typing.py --work <dir> --kind typing|open` covers
typing and the 10 MB file with the IDE's bundled performance-testing plugin: a
script of commands runs when the project opens (`%waitForSmart`,
`%openFile -file big.py -unwfca`, `%goto 50000 1`, `%sleep 3000`, then three
times `%delayType 50|<200 characters of a comment>` and `%sleep 2000`, then
`%exitApp true`; for the 10 MB file `%openFile -file huge.py -unwfca`), and the
IDE writes its spans to an OpenTelemetry JSON file
(`-Didea.diagnostic.opentelemetry.file`). The typing numbers are the IDE's own
typing latency spans: from the key event to the end of the editor's paint for
it, without waiting for the display. The 10 MB number is the `openFile` span.
Runs: the IDE frontmost, on a temporary copy of the configuration, load average
about 3.

### Results (reference)
| date | machine | version | metric | median | worst | notes |
|---|---|---|---|---|---|---|
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | small project, first open, fresh caches: indexed | 8.0 s | - | 1 run; includes attaching bundled shared indexes |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | small project, warm start: ready | 3.46 s | 5.51 s | 5 runs |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | Flask, first open (IDE caches warm): ready / indexed | 1.7 s / 5.6 s | - | 1 run |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | Flask, warm start: ready | 2.60 s | 3.42 s | 3 runs |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | Flask, idle 2 min after indexing: memory | 812 MB RSS (process tree), 643 MB JVM heap used | - | -Xmx2048m |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | typing in the 100k-line file at line 50,000, 3 runs x 200 keys (key to end of paint) | p90 2 / 1 / 2 ms, mean 1 / 1 / 1 ms | max 9 / 7 / 4 ms | per run; `pycharm_typing.py --kind typing` |
| 2026-10-11 | mac-1 | PyCharm 2026.1.3 | open the 10 MB file (`huge.py`, 10,485,700 bytes): `openFile` span | 617 ms | 625 ms | runs 570 / 625 / 617 ms; the very first open took 1031 ms |
| | | | completion | | | not measured yet |

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

### Editor and shell
Measured with `python3 scripts/perf/pyoxide_bench.py --work <dir> --runs 3` on
the files `pycharm_baseline.py` generates (the same as the reference runs). It
runs the release binary (`cargo build --release -p pyoxide-ide`) with
`PYOXIDE_BENCH`; the measurement itself runs inside the application
(`crates/platform/workbench/src/bench.rs`):

- Typing: the 100,000-line file opens, the caret goes to line 50,000,
  column 1, after 3 s the same 200 characters as the reference run are typed
  three times, one key every 50 ms plus a pseudo-random 0-16 ms (so keys do
  not keep one phase to the display's frames), 2 s between runs. Keys go
  through the window's own key dispatch, the path of a key event from the
  platform. Frame start is the start of the root view's render, frame end
  the end of the paint of a canvas painted last; the scene is then handed
  to the GPU (Metal encoding and presentation are not included).
- Two numbers per key:
  1. **Budget metric, key to frame**: from the moment the key is due to the
     end of the paint of the first frame that shows it. It includes timer
     lateness, key handling, any wait for the frame and the frame's own
     work.
  2. **App work, comparable to the reference**: the key handling plus the
     layout and paint of that frame, without the wait between them.
- Open: the 10 MB file is opened from the open project (3 s after the tree
  is listed); time from the open command to the end of the first frame
  that paints its text, and to the first frame painted with its
  highlighting current. Then the view scrolls 3 lines every 8 ms for 5 s:
  frame work and the gaps between frames.
- The window opens without taking the focus (`PYOXIDE_BACKGROUND=1`) and is
  kept above other windows while measuring, so it is not covered (a covered
  window gets no frames from the display; the measurement reports
  `"frames": "forced"` when it has to draw frames itself, and those runs do
  not count for the budget metric).

Machine `mac-1` (Apple M5, 10 cores, 16 GB, macOS 27.0.1), Rust 1.99.0,
release build. Load average about 3 at the start of each run (no build
running). The display refreshed at 60 Hz during these runs (median frame gap
16.7 ms while scrolling).

| date | machine | metric | median | worst | notes |
|---|---|---|---|---|---|
| 2026-10-11 | mac-1 | start to first frame (warm, small project) | 116 ms | 133 ms | 3 runs, from `main` to the end of the first frame's paint |
| 2026-10-11 | mac-1 | typing, 100k-line file, key to frame (budget metric) | p50 3.06 ms, p90 4.34 ms, p99 5.75 ms | max 6.35 ms | 3 runs x 600 keys; per run p99 5.70 / 5.80 / 5.75 ms |
| 2026-10-11 | mac-1 | typing, 100k-line file, app work (comparable to the reference) | p50 1.15 ms, p90 1.37 ms, p99 1.97 ms | max 3.92 ms | per run p90 1.35 / 1.37 / 1.37 ms, mean 1.18 / 1.19 / 1.16 ms, max 3.92 / 3.47 / 3.39 ms; key handling alone p50 0.05 ms |
| 2026-10-11 | mac-1 | open the 10 MB file: first painted text | 24.3 ms | 24.9 ms | 3 runs; reading, decoding and building the rope run off the UI thread |
| 2026-10-11 | mac-1 | open the 10 MB file: highlighting current | 252.4 ms | 252.7 ms | the first full parse runs in the background; the text is usable before |
| 2026-10-11 | mac-1 | scrolling the 10 MB file, 3 lines per frame | frame work p50 3.4 ms, p99 4.4 ms | max 6.8 ms | 300 frames in 5 s at 60 Hz, no gap over 1.5x the median |
| 2026-10-11 | mac-1 | memory after typing (100k-line file open) | 182 MB RSS | 182 MB | single process |
| 2026-10-11 | mac-1 | memory after opening and scrolling the 10 MB file | 265 MB RSS | 266 MB | |
| 2026-10-12 | mac-1 | typing, 100k-line file, with the lexical highlighting layer: key to frame | p50 2.61 ms, p90 3.89 ms, p99 5.45 ms | max 5.88 ms | 2 runs x 600 keys; load average about 4.8 (other applications busy) |
| 2026-10-12 | mac-1 | typing, same runs: app work | p50 0.81 ms, p90 0.91 ms, p99 1.34 ms | max 3.79 ms | per run p99 1.18 / 1.49 ms |
| 2026-10-13 | mac-1 | typing, 100k-line file, key to frame, while a Python process floods the Run console | p50 3.25 / 3.47 ms, p90 4.51 / 4.76 ms, p99 6.09 / 6.17 ms | max 6.87 / 16.27 ms | 2 runs x 600 keys; the file itself runs (`PYOXIDE_RUN='[["run.run",{}]]'`) and prints coloured 90-character lines without pause for 40 s, about a million lines a second through the 1024 KB cycle buffer; the same session without the run: p50 2.84, p99 5.71, max 6.90 ms; load average about 6 |
| 2026-10-13 | mac-1 | typing while the console floods: app work | p50 1.21 / 1.23 ms, p99 2.54 / 2.53 ms | max 2.77 / 13.95 ms | the console takes output at most every 100 ms while it keeps coming; 599 MB RSS at the end of the flooded run (251 MB without the run) |

Against the budgets of docs/performance.md: keystroke to paint p99 5.8 ms
(budget 16 ms); 10 MB file 24 ms to the first text (budget 200 ms), no dropped
frames while scrolling; start-up 116 ms (budget 300 ms, measured warm). The
app's work per key is about the reference IDE's (p90 1.4 ms against 1-2 ms,
max 3-4 ms against 4-9 ms).

Frames normally follow the display's refresh. Drawing only at the next
refresh after a key gives, at 60 Hz, key-to-frame p50 11.0 ms, p90 18.0 ms,
p99 19.5 ms, max 20.6 ms (3 runs, same app work): the wait for the refresh
dominates and the budget is missed. The window therefore draws at once after
every keystroke (macOS: the layer's display callback), so the typed text is
ready for the next refresh; the table above is measured that way.

### Find in file
Engine: `cargo test --release -p search --test perf -- --ignored --nocapture`
on the generated 10 MB file (10,485,700 bytes, 108,100 lines), median of 5
runs. In the application: `PYOXIDE_BACKGROUND=1 PYOXIDE_BENCH=find:<query>`
clears the find field and types the query one key every 150 ms, 5 runs; time
from key dispatch to the end of the first painted frame that shows that
prefix's matches (display frames). Files up to 256 KB are searched on the UI
thread; larger ones on a background snapshot, cancelled by the next key.

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-12 | mac-1 | engine, 10 MB, plain text, 108,100 matches | 4.45 ms | char offsets for them 1.2 ms; no match 1.45 ms |
| 2026-10-12 | mac-1 | engine, 10 MB, regex `'a+'$`, 108,100 matches | 8.6 ms | |
| 2026-10-12 | mac-1 | engine, 10 MB, 1,000,000 match limit | 19.2 ms | worst case (Words rejecting every candidate) 150 ms, cancellable; cancel returns within 1.6 ms |
| 2026-10-12 | mac-1 | key to highlight, 200-line file | p50 4.05 ms, p90 4.69 ms, p99 5.43 ms | key handling 0.3-0.5 ms |
| 2026-10-12 | mac-1 | key to highlight, 100k lines (1.6 MB) | p50 16.1 ms, p90 23.5 ms, p99 25.2 ms | background search plus the hop back to the UI thread |
| 2026-10-12 | mac-1 | key to highlight, 10 MB | p50 21.5 ms, p90 22.9 ms, p99 27.8 ms | query `x = 'aaa`; a query with no matches (`import`): p50 10.0 ms, p90 18.7 ms |

### Find in files
In the application: `PYOXIDE_BACKGROUND=1 PYOXIDE_BENCH=find-in-files:<query>`
opens the project, then runs the search five times through the
`find-in-files.search` command (no typing delay); time from the command to
the first rows on the UI thread and to the end of the search, and the
longest frame drawn meanwhile. Flask 3.1.0 (shallow clone); a generated tree
of 20,000 Python files in 400 folders, 300 MB (scratch script, not in the
repository). Warm file cache; the search runs on all cores and stops at 100
matches as the reference does.

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-12 | mac-1 | Flask, `redirect` (stops at 100): first rows / end | p50 1.0 ms / 4.7 ms | p90 1.2 ms / 6.2 ms; "100+ matches in 22+ files" |
| 2026-10-12 | mac-1 | Flask, no match (every file searched): end | p50 6.4 ms | p90 16.7 ms; longest frame 7.7 ms |
| 2026-10-12 | mac-1 | 20k files / 300 MB, 10 matches in 10 files: first rows / end | p50 114 ms / 252 ms | p90 201 ms / 368 ms; longest frame meanwhile 8.4 ms; 102 MB RSS after the runs |
| 2026-10-12 | mac-1 | 20k files / 300 MB, `cache` (stops at 100): first rows / end | p50 0.9 ms / 1.9 ms | |

### Go to File
Engine: `FILE_INDEX_BENCH=<dir> cargo test --release -p file-index bench --
--ignored --nocapture` builds the index of a folder (median of 10 builds) and
types the words of `FILE_INDEX_BENCH_TYPE` one char at a time (median of 20
searches with the final sort per prefix). In the application: `PYOXIDE_BACKGROUND=1 PYOXIDE_BENCH=go-to-file:<query>`
opens the project, waits for its index, then 5 times opens Go to File with an
empty field and types the query one key every 150 ms; time from key dispatch
to the end of the first painted frame that shows that prefix's rows (display
frames), which includes the popup's 100 ms pause after a key, and the search
alone (on a background thread, to the rows on the UI thread). Flask 3.1.0
and the generated 20,000-file tree of the Find in Files rows.

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-12 | mac-1 | engine, Flask (301 entries): index build | p50 2.9 ms | |
| 2026-10-12 | mac-1 | engine, Flask: a key while typing | p50 0.06 ms, max 0.2 ms | |
| 2026-10-12 | mac-1 | engine, 20k files (20,450 entries): index build | p50 36 ms | |
| 2026-10-12 | mac-1 | engine, 20k files: a key while typing | p50 0.17-3.5 ms, max 8.9 ms | `pkg0`, 13.5k results |
| 2026-10-12 | mac-1 | Flask, `flaskapp`: index build in the application | 2.2 ms | |
| 2026-10-12 | mac-1 | Flask, `flaskapp`: key to rows / search | p50 115 ms / 0.26 ms | p90 127 ms / 0.36 ms; the 100 ms pause included |
| 2026-10-12 | mac-1 | 20k files, `mod_1234`: index build in the application | 40 ms | 117 MB RSS after the runs |
| 2026-10-12 | mac-1 | 20k files, `mod_1234`: key to rows / search | p50 131 ms / 16 ms | p90 146 ms / 22 ms; `m` matches every file |
| 2026-10-12 | mac-1 | 20k files, `pkg0sub`: key to rows / search | p50 128 ms / 1.3 ms | p90 141 ms / 26 ms |
| 2026-10-13 | mac-1 | Flask, `flaskapp`: key to rows / search | p50 125 ms / 0.29 ms | p90 127 ms / 0.42 ms; every match ranked by the popup's rule, 15 rows |
| 2026-10-13 | mac-1 | 20k files, `mod_1234`: key to rows / search | p50 143 ms / 23 ms | p90 146 ms / 29 ms; ranking every match (`m` matches every file) |
| 2026-10-13 | mac-1 | 20k files, `pkg0sub`: key to rows / search | p50 127 ms / 1.4 ms | p90 157 ms / 31 ms |

### Terminal sessions
`cargo test -p terminal --release --test throughput -- --ignored --nocapture`:
16 MiB of 77-byte lines from the test helper through the pseudo-terminal
into the emulator (120x40, 10,000 lines of scrollback) while a view takes a
snapshot at most once per 16.7 ms frame, spawn to exit; and the same bytes
parsed by the emulator alone. CI runs both on every OS (debug build) and
prints the numbers in the log. The machine was busy with other builds
(load average about 50), so these are lower bounds.

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-16 | mac-1 | emulator alone | 66-81 MiB/s | `alacritty_terminal` 0.26.0 with the OSC pre-scan |
| 2026-10-16 | mac-1 | through the PTY, snapshots per frame | 20-25 MiB/s | macOS openpty; without snapshots 36 MiB/s |
| 2026-10-16 | mac-1 | a snapshot during the flood (lock wait and copy) | 0.26-6 ms | 0.2 ms without output; 5 ms before the reader released the lock fairly |
| 2026-10-16 | mac-1 | emulator alone (rerun, load average 5-15) | 168 MiB/s | release build |
| 2026-10-16 | mac-1 | through the PTY, snapshots per frame (rerun) | 42 MiB/s | a snapshot of 120x40 in 0.22 ms |

### Terminal view (headless)
`cargo test --release -p terminal-ui --test perf -- --ignored --nocapture --test-threads=1`,
no window: the work of a frame before painting (the snapshot and the plan of
backgrounds, text runs and the cursor), `cat` of a 16 MiB file of 85-byte
lines with SGR colours through the pseudo-terminal with a snapshot and a plan
per 16.7 ms frame, and a key written to `cat` to the snapshot that shows its
echo (three runs, release build, load average 4-15). `bench_flood_frame_work`:
`cat` of 128 MiB while a loop draws 60 frames a second and, when the screen
changed, takes it and plans a frame; "in the frame" copies the screen there
under the terminal's lock, "on a thread" takes the copies a background
thread made at the view's rates (one per frame, one per 100 ms after 200 ms
of output), as the view does since 2026-10-17. Frame work is the frame
thread's time for the copy (or taking it) and the plan.

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-16 | mac-1 | plan of a 120x40 frame | 90-93 us | a busy listing: 679 text runs, 76 background quads |
| 2026-10-16 | mac-1 | plan of a 240x70 frame | 260-297 us | 2333 text runs, 258 background quads |
| 2026-10-16 | mac-1 | `cat` 16 MiB, snapshot and plan per frame | 124-151 MiB/s | snapshot 0.14-0.22 ms, plan 0.05 ms per frame |
| 2026-10-16 | mac-1 | key to snapshot through the PTY | p50 0.06-0.10 ms, p99 0.13-0.15 ms | 200 keys |
| 2026-10-17 | mac-1 | flood of 128 MiB, copy in the frame | 112-153 MiB/s; frame work p50 0.08-0.14 ms, p99 0.42-0.61 ms | 50-68 frames with a new screen; load average 7 |
| 2026-10-17 | mac-1 | flood of 128 MiB, copy on a thread | 111-136 MiB/s; frame work p50 0.05-0.06 ms, p99 0.07-0.12 ms | 19-21 frames with a new screen (ten a second once flooding); load average 7 |
| 2026-10-17 | mac-1 | `cat` 16 MiB, snapshot and plan per frame (rerun) | 97.5 MiB/s | snapshot 0.14 ms, plan 0.06 ms per frame; load average 10 |
| 2026-10-17 | mac-1 | plan of a 120x40 / 240x70 frame (rerun) | 82 us / 291 us | |
| 2026-10-17 | mac-1 | key to snapshot through the PTY (rerun) | p50 0.02 ms, p99 0.04 ms | 200 keys |

### Terminal view (window)
`PYOXIDE_BENCH=terminal-typing|terminal-flood:64|terminal-scroll`, and the
editor's `PYOXIDE_BENCH=typing` at line 50,000 of a 150,000-line file with and
without `PYOXIDE_BENCH_FLOOD=1` (a terminal printing a 4 MiB file again and
again in the development terminal), release build, display frames, 600 keys
(three runs of 200, one every 50-67 ms). Key to frame is the time from the
key to the end of the first frame that shows it. These runs are of the view
that copied the screen in the frame on the UI thread, at most once per
16 ms, and redrew the window on every copy; other builds were running (load
averages per row). The flood's time to the last line was not recorded (the
measurement looked for the last line below the shell's prompt).

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-16 | mac-1 | terminal: key to frame of the echo | p50 22.4 ms, p90 29.5 ms, p99 34.3 ms, max 45.7 ms | 12 of 600 keys without a frame of their own; frame work p50 1.9 ms; load average 6 |
| 2026-10-16 | mac-1 | terminal: `cat` of 64 MiB, frames meanwhile | 85 frames, frame work p50 2.2 ms, p90 12.3 ms, p99 28.8 ms | load average 42; RSS 149 MB |
| 2026-10-16 | mac-1 | terminal: wheel over 2 MiB of scrollback | frame work p50 1.6 ms, p99 2.6 ms; gaps p50 16.7 ms, p99 18.2 ms, none over 1.5x the median | 361 frames; load average 41 |
| 2026-10-16 | mac-1 | editor typing, no terminal | key to frame p50 3.9 ms, p99 6.7 ms, max 12.3 ms | key handling p50 0.1 ms; load average 39 |
| 2026-10-16 | mac-1 | editor typing while a terminal floods | key to frame p50 6.1 ms, p99 28.0 ms, max 75.9 ms | key handling p50 2.4 ms, p99 13.5 ms (GPUI draws the window the terminal marked before it dispatches a key); load average 21 |

Rerun of the same commands on 2026-10-17, each a single run of 600 keys or
one flood, with timing probes that split the key's way into the shell's echo,
the wakeup to the copy and the copy to the end of the frame that shows it.
"Copy in the frame" is the view of the rows above; "copy on a thread" copies
the screen on a background thread (one per 16 ms, one per 100 ms after output
kept coming for 200 ms) and still drew the window at every key; "final" does
not draw the window at a key but when the echo's copy lands (the next frame
while flooding), so the frame shown at the next refresh is the one with the
echo. Load average is the one-minute value at the start of the run; the
machine ran other builds.

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-17 | mac-1 | terminal: key to frame of the echo, copy in the frame | p50 20.8 ms, p90 27.4 ms, p99 33.0 ms, max 36.6 ms | 12 of 600 keys showed no frame of their own: the bench pressed Enter right after key 50 (it waits one interval since); wakeup to copy p50 17.1 ms; load average 2.7 |
| 2026-10-17 | mac-1 | terminal: key to frame of the echo, copy on a thread | p50 20.6 ms, p90 27.6 ms, p99 33.6 ms, max 35.1 ms | the same 12 keys (the bench); copy to the end of the frame p50 17.3 ms: the window drawn at the key was shown at the refresh before the echo was back, and the echo waited for the next one; load average 3.2 |
| 2026-10-17 | mac-1 | terminal: key to frame of the echo, final | p50 2.85 ms, p90 4.15 ms, p99 5.49 ms, max 6.2 ms | every key has a frame; echo p50 0.08 ms, copy to the end of the frame p50 1.05 ms; editor without a terminal in the same run set: p50 3.94 ms; load average 4.9 |
| 2026-10-17 | mac-1 | terminal: `cat` of 64 MiB, copy in the frame | 132 MiB/s, 17 frames, frame work p50 1.13 ms, p99 4.93 ms | load average 2.4 |
| 2026-10-17 | mac-1 | terminal: `cat` of 64 MiB, copy on a thread | 105 MiB/s, 16 frames (15 copies), frame work p50 1.05 ms, p99 5.43 ms | load average 3.4 |
| 2026-10-17 | mac-1 | terminal: `cat` of 64 MiB, final | 69.7 MiB/s, 19 frames (18 copies), frame work p50 1.35 ms, p99 2.91 ms; RSS 163 MB | budget: >= 50 MiB/s and ten repaints a second; load average 4.6; the rate varies between runs with the load |
| 2026-10-17 | mac-1 | terminal: wheel over 2 MiB of scrollback, copy in the frame | frame work p50 0.80 ms, p99 0.97 ms; gaps p50 16.67 ms, p99 17.91 ms, none over 1.5x the median | 361 frames; load average 2.4 |
| 2026-10-17 | mac-1 | terminal: wheel over 2 MiB of scrollback, final | frame work p50 1.13 ms, p99 1.27 ms; gaps p50 16.68 ms, p99 17.65 ms, none over 1.5x the median | 361 frames; load average 5.5 |
| 2026-10-17 | mac-1 | editor typing, no terminal | key to frame p50 3.79 ms, p99 6.25 ms (copy in the frame); p50 3.94 ms, p99 6.58 ms (final) | load average 2.4 and 5.5 |
| 2026-10-17 | mac-1 | editor typing while a terminal floods, copy in the frame | key to frame p50 4.18 ms, p99 7.02 ms, max 7.84 ms | key handling p50 1.5 ms (the window the terminal marked is drawn before the key is dispatched); load average 2.8 |
| 2026-10-17 | mac-1 | editor typing while a terminal floods, final | key to frame p50 4.24 ms, p99 7.20 ms, max 8.57 ms | p99 0.62 ms above typing without a terminal (budget: 2 ms); key handling p50 0.14 ms; load average 5.3 |

### Git status and line ranges
Engine measurements (ADR 0016 budgets), release builds, git 2.56.0, other
builds running on the machine. Status: a generated repository of 10,000
committed files in 100 folders with 5 files changed;
`cargo test --release --workspace --test perf -- --ignored --nocapture`
(`crates/platform/git/tests/perf.rs`), medians of 7 runs (5 for the
service). "Watcher batch to snapshot" is the time from a reported batch to
the merged snapshot minus the 300 ms debounce: the limited status of the
dirty path, the readers and the merge. Line ranges: a 10,000-line file with
175 changed ranges, `cargo test -p vcs --release perf -- --ignored
--nocapture`, median of 9.

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-16 | mac-1 | 10k files: full status | 42 ms | one `status --porcelain=v2` with untracked and ignored files |
| 2026-10-16 | mac-1 | 10k files: status limited to 5 paths | 26 ms | |
| 2026-10-16 | mac-1 | 10k files: watcher batch to snapshot (after the debounce) | 25 ms | budget 150 ms |
| 2026-10-16 | mac-1 | 10,000 lines: line ranges against the base | 1.8 ms | budget 50 ms after the last keystroke |
| 2026-10-16 | mac-1 | 10,000 lines: ranges shifted by an edit | 0.9 µs | until the next computation |

### Change markers
`cargo test -p workbench --release --test version_control perf -- --ignored
--nocapture` (`crates/platform/workbench/tests/version_control.rs`): a
committed file of 10,000 lines opened in the workbench, a character typed
at a different line each time; "keystroke to markers" is the time from the
simulated key to the markers drawn by the editor's decorations, including
the range computation of the whole text off the UI thread (the edited
lines are marked at once). Median of 9.

| date | machine | metric | value | notes |
|---|---|---|---|---|
| 2026-10-16 | mac-1 | 10,000 lines: keystroke to markers | 4.0 ms | budget 50 ms after the last keystroke |
| 2026-10-17 | mac-1 | 10,000 lines: keystroke to markers | 4.6 ms | same build rebased; load average about 58 from other builds |
| 2026-10-17 | mac-1 | 10,000 lines: edit to ranges drawn | 3.0 ms | the part after the edit: the whole-text ranges and the redraw (9 ranges) |

Not measured yet: idle memory of a mid-size project (no indexing exists yet
to compare with the reference's 812 MB), cold start after purging the file
caches, Windows.
