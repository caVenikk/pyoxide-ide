use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command};

use crate::ExitStatus;

/// What interrupt and kill need: the process id, which is also the id of the
/// process group the process leads.
pub(crate) struct OsProcess {
    pid: libc::pid_t,
}

impl OsProcess {
    pub(crate) fn new(child: &Child) -> io::Result<Self> {
        let pid = libc::pid_t::try_from(child.id())
            .map_err(|_| io::Error::other("process id out of range"))?;
        Ok(OsProcess { pid })
    }
}

/// Starts the process as the leader of a new process group, so that a kill
/// of the group reaches descendants that lost their parent.
///
/// `SIGINT` is reset to its default action in the child. An ignored signal
/// stays ignored across `exec`, and the application itself ignores `SIGINT`
/// when it was started as a background job of a non-interactive shell; the
/// program would then not stop on an interrupt (Python does not install its
/// `KeyboardInterrupt` handler when `SIGINT` is ignored at startup).
pub(crate) fn spawn(command: &mut Command) -> io::Result<Child> {
    command.process_group(0);
    // SAFETY: the closure runs between `fork` and `exec` and calls only
    // `signal`, which is async-signal-safe, and does not allocate.
    unsafe {
        command.pre_exec(|| {
            if libc::signal(libc::SIGINT, libc::SIG_DFL) == libc::SIG_ERR {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}

pub(crate) type Pipe = RawFd;

pub(crate) fn pipe_of(pipe: &impl AsRawFd) -> Pipe {
    pipe.as_raw_fd()
}

/// Bytes waiting in a pipe, `None` when unknown.
pub(crate) fn pipe_available(fd: Pipe) -> Option<usize> {
    let mut n: libc::c_int = 0;
    // SAFETY: FIONREAD writes one int; `fd` is open while the reader that
    // owns it has not finished, which the caller checks under a lock.
    let r = unsafe { libc::ioctl(fd, libc::FIONREAD, &mut n) };
    (r == 0).then(|| usize::try_from(n).unwrap_or(0))
}

pub(crate) fn exit_status(status: std::process::ExitStatus) -> ExitStatus {
    ExitStatus {
        code: status.code(),
        signal: status.signal(),
    }
}

/// `SIGINT` to the process tree: descendants first, then the process (the
/// reference's `UnixProcessManager.sendSignalToProcessTree`).
pub(crate) fn interrupt(process: &OsProcess) -> io::Result<()> {
    signal_tree(process.pid, libc::SIGINT)
}

/// `SIGKILL` to the process tree (as the reference's
/// `OSProcessUtil.killProcessTree`), then to the process group, which also
/// reaches descendants whose parent already exited.
pub(crate) fn kill(process: &OsProcess) -> io::Result<()> {
    let result = signal_tree(process.pid, libc::SIGKILL);
    // SAFETY: plain syscall; a negative pid addresses the process group the
    // process leads.
    unsafe { libc::kill(-process.pid, libc::SIGKILL) };
    result
}

fn signal_tree(root: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    for pid in descendants_deepest_first(root) {
        // SAFETY: plain syscall.
        unsafe { libc::kill(pid, signal) };
    }
    // SAFETY: plain syscall; `root` is our child, not reaped yet or a zombie.
    if unsafe { libc::kill(root, signal) } == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(err)
    }
}

/// The descendants of `root`, children after their own descendants (the
/// reference signals a process after its children).
fn descendants_deepest_first(root: libc::pid_t) -> Vec<libc::pid_t> {
    let children = children_map(root);
    let mut out = Vec::new();
    let mut seen = vec![root];
    fn visit(
        pid: libc::pid_t,
        children: &HashMap<libc::pid_t, Vec<libc::pid_t>>,
        seen: &mut Vec<libc::pid_t>,
        out: &mut Vec<libc::pid_t>,
    ) {
        if let Some(kids) = children.get(&pid) {
            for &kid in kids {
                if seen.contains(&kid) {
                    continue;
                }
                seen.push(kid);
                visit(kid, children, seen, out);
                out.push(kid);
            }
        }
    }
    visit(root, &children, &mut seen, &mut out);
    out
}

/// The direct children of the process `pid`, as the reference lists them
/// for a terminal's shell (`ps -e -o ppid,pid`, the processes whose parent
/// is the shell).
pub(crate) fn child_pids(pid: u32) -> Vec<u32> {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return Vec::new();
    };
    direct_children(pid)
        .into_iter()
        .filter_map(|p| u32::try_from(p).ok())
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn direct_children(pid: libc::pid_t) -> Vec<libc::pid_t> {
    children_map(pid).remove(&pid).unwrap_or_default()
}

#[cfg(target_vendor = "apple")]
fn direct_children(pid: libc::pid_t) -> Vec<libc::pid_t> {
    list_children(pid)
}

/// Other Unix systems: no process table walk (see [`children_map`]).
#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn direct_children(_pid: libc::pid_t) -> Vec<libc::pid_t> {
    Vec::new()
}

/// Parent to children for the processes below `root`.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn children_map(_root: libc::pid_t) -> HashMap<libc::pid_t, Vec<libc::pid_t>> {
    let mut map: HashMap<libc::pid_t, Vec<libc::pid_t>> = HashMap::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return map;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<libc::pid_t>().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `pid (comm) state ppid ...`; comm may contain spaces and parentheses.
        let Some(rest) = stat.rfind(')').map(|i| &stat[i + 1..]) else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let ppid = fields.nth(1).and_then(|s| s.parse::<libc::pid_t>().ok());
        if let Some(ppid) = ppid {
            map.entry(ppid).or_default().push(pid);
        }
    }
    map
}

#[cfg(target_vendor = "apple")]
fn children_map(root: libc::pid_t) -> HashMap<libc::pid_t, Vec<libc::pid_t>> {
    let mut map = HashMap::new();
    let mut queue = vec![root];
    while let Some(pid) = queue.pop() {
        if map.contains_key(&pid) || map.len() > 4096 {
            continue;
        }
        let kids = list_children(pid);
        queue.extend(kids.iter().copied());
        map.insert(pid, kids);
    }
    map
}

#[cfg(target_vendor = "apple")]
fn list_children(pid: libc::pid_t) -> Vec<libc::pid_t> {
    let mut buf: Vec<libc::pid_t> = vec![0; 1024];
    let size = libc::c_int::try_from(buf.len() * std::mem::size_of::<libc::pid_t>())
        .unwrap_or(libc::c_int::MAX);
    // SAFETY: the buffer is `size` bytes long and zeroed; the result counts
    // pids (or bytes on old systems), so the zero entries are skipped below.
    let n = unsafe { libc::proc_listchildpids(pid, buf.as_mut_ptr().cast(), size) };
    if n <= 0 {
        return Vec::new();
    }
    buf.retain(|p| *p > 0);
    buf
}

/// Other Unix systems: no process table walk; the process group covers the
/// descendants that stayed in it.
#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn children_map(_root: libc::pid_t) -> HashMap<libc::pid_t, Vec<libc::pid_t>> {
    HashMap::new()
}
