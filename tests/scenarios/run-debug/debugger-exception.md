# Stop on an exception

## Steps
1. `main.py` calls a function that runs `raise ValueError("x")`; no line
   breakpoints. Debug it.
2. Look at the editor, frames and variables.
3. Press F9.
4. In the Breakpoints dialog switch "Any exception" to On raise and tick
   Ignore library files; debug a script that catches an exception it
   raised.

## Expected
- Step 1: the default "Any exception" breakpoint (On termination) stops
  the program at the `raise` line.
- Step 2: the execution line is highlighted; the frames are the
  traceback's frames; the variables show the exception.
- Step 3: the Console prints the traceback with links to the files and
  "Process finished with exit code 1".
- Step 4: the program stops at the `raise` even though the exception is
  caught; exceptions raised inside libraries do not stop it.

## Commands
`debug.current-file`, `debug.resume`, `debug.view-breakpoints`,
`debug.state` (stop reason, exception).

## Source
https://www.jetbrains.com/help/pycharm/using-breakpoints.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `PyExceptionBreakpointType`;
the DAP `setExceptionBreakpoints` filters `uncaught` and `raised` of
debugpy.
