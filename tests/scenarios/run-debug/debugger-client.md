# Debugger engine: the DAP client and sessions

Engine-level behaviour without a UI, driven by a scripted adapter (the
`dap-fake-adapter` binary, or `dap_client::fake::adapter` in a test's
process) in place of debugpy. The same functions back the `debug.*`
commands.

## Steps
1. Start a session on a project with two line breakpoints (one with a
   condition), one disabled breakpoint and the default exception
   breakpoint; the adapter answers `initialize`, receives `launch`, sends
   `initialized`, and only after the breakpoints and `configurationDone`
   answers `launch`.
2. The adapter asks `runInTerminal` for the program; it stops at the first
   breakpoint.
3. Read the state; open the frames, the scopes of the top frame, the
   children of a list; change an item; evaluate `x + 1`.
4. Add a breakpoint in another file, mute and unmute the breakpoints, add
   a temporary breakpoint and hit it.
5. Resume until the program exits.
6. Stop a session that is suspended; stop one whose adapter does not
   answer `terminate`.
7. Drop a request before the adapter answers.
8. The adapter prints a traceback to its standard error and exits with
   code 7 during `initialize`.
9. Open a project whose `.idea/workspace.xml` has `XDebuggerManager`
   breakpoints and watches; reopen it after removing one breakpoint.

## Expected
- Step 1: the session goes Starting, then Running once the configuration
  is sent and `launch` is answered; the disabled breakpoint is not sent;
  the breakpoints keep what the adapter said (verified, the line it
  moved them to).
- Step 2: the program's process is started from the `runInTerminal`
  request (working directory, arguments, environment) and its output is
  available as the Debug console, once; the process id is the answer to
  the adapter. The session is Suspended with the thread, the frames in
  pages (`totalFrames` kept), the stop reason and the hit breakpoint.
- Step 3: scopes and children load on request, in pages; the item
  changes (`setVariable`); the evaluation answers with the value.
  `debug.state` shows the session, frames, variables and the breakpoints
  with their conditions and the adapter's lines.
- Step 4: only the files whose breakpoints changed are sent again; muted,
  every file is sent with no breakpoints and the exception filters are
  empty; unmuted, everything comes back. The temporary breakpoint is
  removed once it is hit.
- Step 5: the session ends with the exit code 0 and no error; the console
  process has ended.
- Step 6: `terminate`, then `disconnect {terminateDebuggee: true}`; an
  adapter that does not answer within the timeout is killed. Nothing is
  left running.
- Step 7: the request is cancelled (`cancel` where the adapter supports
  it) and the late answer is ignored.
- Step 8: the session ends with the error "... exited with code 7" and
  the tail of the adapter's log ("boom"); requests still waiting fail
  with a closed-connection error, not a second failure message.
- Step 9: the breakpoints, the default exception breakpoint (once) and the
  watches of their configuration are imported once, with a marker in the
  personal state; the removed breakpoint stays removed; `.idea` is not
  changed.

## Commands
`debug.state`, `debug.toggle-breakpoint {"file": ..., "line": ...}`,
`debug.toggle-breakpoint-enabled`, `debug.edit-breakpoint`,
`debug.add-exception-breakpoint {"class": ...}`, `debug.mute-breakpoints`,
`debug.new-watch`, `debug.remove-watch`, `debug.resume`, `debug.pause`,
`debug.step-over`, `debug.step-into`, `debug.step-out`, `debug.stop`,
`debug.evaluate`, `debug.set-value`.

## Source
The Debug Adapter Protocol specification,
https://microsoft.github.io/debug-adapter-protocol/specification
(initialization sequence, `runInTerminal`, `cancel`, `terminate`,
`disconnect`); IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: the
`XDebuggerManager` component of `.idea/workspace.xml` in
`platform/xdebugger-impl`. The tests are `crates/platform/dap-client/tests/
fake_adapter.rs` and `crates/platform/debugger/tests/session.rs`.
