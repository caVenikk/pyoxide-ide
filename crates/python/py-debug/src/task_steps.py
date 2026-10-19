# Steps that stay in their asyncio task, for pydevd's sys.monitoring mode.
#
# Run once in each debugged process by an evaluation in pydevd (see
# `launch::task_steps`), with `d` the pydevd debugger; `result` says what it
# did.
#
# With Python's trace function (before 3.12) pydevd keeps a step that starts
# in a coroutine in that coroutine (CMD_STEP_INTO_COROUTINE in
# `pydevd_frame.py`). Its sys.monitoring stepping (3.12 and later) checks no
# such context: a step over an `await` stops at the next line any task of the
# event loop runs, such as a background task polling with `asyncio.sleep`.
# This wraps the debugger's `set_suspend` and `do_wait_suspend`: a step that
# starts in an asyncio task does not stop in another task while that one has
# not finished; breakpoints, exceptions and pauses stop as before.
#
# Parses with Python 3.8, the oldest one debugpy debugs.
import sys
import threading

from _pydevd_bundle import pydevd_comm_constants as commands
from _pydevd_bundle import pydevd_constants

if not getattr(pydevd_constants, "PYDEVD_USE_SYS_MONITORING", False):
    result = "trace"
elif getattr(d, "_pyoxide_task_steps", False):
    result = "installed"
else:
    _steps = frozenset(
        getattr(commands, name)
        for name in (
            "CMD_STEP_INTO",
            "CMD_STEP_INTO_MY_CODE",
            "CMD_STEP_INTO_COROUTINE",
            "CMD_STEP_OVER",
            "CMD_STEP_OVER_MY_CODE",
            "CMD_STEP_RETURN",
            "CMD_STEP_RETURN_MY_CODE",
            "CMD_SMART_STEP_INTO",
        )
        if hasattr(commands, name)
    )
    _suspended = pydevd_constants.STATE_SUSPEND
    # The task a step started in, by thread.
    _tasks = {}
    # Threads whose step stop was dropped: their next `do_wait_suspend`
    # returns at once.
    _dropped = set()
    _set_suspend = d.set_suspend
    _do_wait_suspend = d.do_wait_suspend

    def _current_task():
        asyncio = sys.modules.get("asyncio")
        if asyncio is None:
            return None
        try:
            return asyncio.current_task()
        except RuntimeError:
            # No event loop runs on this thread.
            return None

    def _outside(thread, stop_reason):
        ident = thread.ident
        task = _tasks.get(ident)
        if task is None or stop_reason not in _steps or ident != threading.get_ident():
            return False
        return not task.done() and _current_task() is not task

    def set_suspend(thread, stop_reason, *args, **kwargs):
        try:
            outside = _outside(thread, stop_reason)
        except Exception:
            outside = False
        if outside:
            _dropped.add(thread.ident)
            return None
        _tasks.pop(thread.ident, None)
        _dropped.discard(thread.ident)
        return _set_suspend(thread, stop_reason, *args, **kwargs)

    def do_wait_suspend(thread, *args, **kwargs):
        ident = thread.ident
        info = getattr(thread, "additional_info", None)
        if ident in _dropped:
            _dropped.discard(ident)
            # Nothing suspended the thread since: the stop was dropped.
            if info is not None and info.pydev_state != _suspended:
                return None
        answer = _do_wait_suspend(thread, *args, **kwargs)
        # The thread moves on; a step keeps to the task it starts in.
        task = None
        if info is not None and info.pydev_step_cmd in _steps:
            try:
                task = _current_task()
            except Exception:
                task = None
        if task is None:
            _tasks.pop(ident, None)
        else:
            _tasks[ident] = task
        return answer

    d.set_suspend = set_suspend
    d.do_wait_suspend = do_wait_suspend
    d._pyoxide_task_steps = True
    result = "installed"
