- crates/platform/terminal/src/io_loop.rs
  Closely adapted from alacritty_terminal 0.26.0 (Apache-2.0, (c) The
  Alacritty Project), https://github.com/alacritty/alacritty, crate
  alacritty_terminal 0.26.0: src/event_loop.rs (the PTY read loop that
  parses in batches under the terminal's lock, taken unfairly by the
  reader and forced at a full buffer; the queued writes; the poller
  registration with write interest; the child exit with a final read).
  Changes: no ref-test recording; the output is pre-scanned for OSC 7,
  OSC 133 and Pyoxide's own events and parsed in pieces at their ends;
  hang-up, kill and shutdown with a grace period; the lock is released
  fairly after each batch; the PTY is resized with its own ioctl on Unix.
