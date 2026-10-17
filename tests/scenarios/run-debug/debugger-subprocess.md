# Stop in a subprocess (a web server with a reloader)

## Steps
1. A project with FastAPI and uvicorn in its environment, `api.py`:
   ```python
   import os

   from fastapi import FastAPI

   app = FastAPI()


   @app.get("/hello")
   async def hello():
       pid = os.getpid()
       return {"pid": pid}
   ```
   and `main.py`:
   ```python
   import uvicorn

   if __name__ == "__main__":
       uvicorn.run("api:app", port=8000, reload=True)
   ```
   Set a breakpoint on `pid = os.getpid()`.
2. Debug `main.py` (Ctrl+D on macOS, Shift+F9 otherwise).
3. Open `http://localhost:8000/hello` in a browser.
4. Press F8, then F9.
5. Press Stop (Cmd+F2 on macOS, Ctrl+F2 otherwise).

The same for `python -m uvicorn api:app --reload` (a module run
configuration), `python -m fastapi dev api.py` and for a run
configuration with "Run with uv run" on.

## Expected
- Step 2: the server starts; its reloader is the program, the server is
  a subprocess the debugger attaches to (Settings, Python, Debugger,
  "Attach to subprocess automatically while debugging", on by default).
- Step 3: the session stops in the server's process: the frames start
  with `hello, api.py:10`, the execution line is highlighted in the
  editor; the browser waits.
- Step 4: the step goes to the next line in the same process; Resume
  lets the request finish, the browser shows the answer.
- Step 5: the reloader, the server and every other process of the
  session end; nothing is left running.

## Commands
`debug.current-file`, `debug.step-over`, `debug.resume`, `run.stop`,
`debug.state` (`subprocesses`: the subprocesses debugged in the session;
the execution point).

## Source
https://www.jetbrains.com/help/pycharm/settings-debugger.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `PyDebuggerOptionsProvider`
(`myAttachToSubprocess`, on), `PyDebugRunner` (`--multiprocess`);
debugpy 1.8.21 `adapter/clients.py` (`notify_of_subprocess`,
`startDebugging`). How the reference lists the threads of several
processes is to verify against a reference screenshot.
