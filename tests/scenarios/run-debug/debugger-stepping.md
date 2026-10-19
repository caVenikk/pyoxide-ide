# Stepping

## Steps
1. Stop at a breakpoint in a function that calls another project
   function and a library function.
2. Press F8 twice.
3. Press F7 on the line with the project call; press Shift+F8.
4. Press F7 on a line with a library call; then Alt+Shift+F7 on the same
   kind of line.
5. Put the caret on a later line and press Alt+F9; set a breakpoint
   between and use Force Run to Cursor (Cmd+Alt+F9 / Ctrl+Alt+F9).
6. Scroll away and press Alt+F10.
7. Press F9 until the end.
8. A FastAPI app (Python 3.12 or later) whose `lifespan` starts a
   background task, `asyncio.create_task(poller())`, a loop of
   `await asyncio.sleep(0.01)`; a breakpoint in an `async def` endpoint
   that awaits a library call (`await asyncio.sleep(0.1)`) and a coroutine
   of the project. Call the endpoint, then press F8, F7, Alt+Shift+F7
   (Step Into My Code) and Shift+F8 on its `await` lines.

## Expected
- Step 2: the execution line moves down; frames and variables update;
  values changed by the step are shown in the changed colour, in the tree
  and inline.
- Step 3: F7 enters the function (a new top frame), Shift+F8 returns to
  the caller.
- Step 4: F7 enters the library code (greyed frame); Step Into My Code
  skips library code and stops in project code.
- Step 5: the program stops at the caret line; Force Run to Cursor
  ignores the breakpoint between.
- Step 6: the editor shows the execution point again.
- Step 7: the session ends; step actions are disabled; "Process finished
  with exit code 0".
- Step 8: every stop is in the endpoint's coroutine, or in the project
  coroutine it awaits (F7 and Step Into My Code enter it, Shift+F8
  returns), on the event loop's thread; never in the poller, which runs
  on the same loop. After the endpoint returns, a step may stop in the
  poller once the request's task has finished.

## Commands
`debug.step-over`, `debug.step-into`, `debug.step-into-my-code`,
`debug.step-out`, `debug.run-to-cursor`, `debug.force-run-to-cursor`,
`debug.show-execution-point`, `debug.resume`, `debug.state`.

## Source
https://www.jetbrains.com/help/pycharm/debugging-code.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `ExecutionActions.xml`,
keymaps in `platform/platform-resources/src/keymaps/`, `PyDebugProcess`.
