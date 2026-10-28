//! Nothing started in a terminal outlives the application (the reference
//! closes the sessions on exit: pty4j hangs the shell up and the kernel
//! passes the hang-up on to the foreground job).
//!
//! The hang-up of a closed pseudo-terminal does not reach every program: one
//! that ignores `SIGHUP` or moved to a process group of its own stays, and
//! the threads that end sessions in the background do not get to run when
//! the process exits. [`end_all_sessions`] therefore ends every live shell
//! and its descendants synchronously on quit: hang-up, a short grace, then a
//! kill of whatever remains. On Windows every shell also joins a job object
//! that kills its processes when the application ends for any reason.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// How long programs get to end after the hang-up before they are killed.
pub const QUIT_GRACE: Duration = Duration::from_millis(400);

/// How often the end of the shells is checked while waiting.
const POLL: Duration = Duration::from_millis(10);

struct Live {
    pid: u32,
    exited: Arc<AtomicBool>,
}

static LIVE: Mutex<Vec<Live>> = Mutex::new(Vec::new());

/// Notes a started shell until its session ends.
pub(crate) fn register(pid: u32, exited: Arc<AtomicBool>) {
    let mut live = LIVE.lock();
    live.retain(|l| !l.exited.load(Ordering::Acquire));
    live.push(Live { pid, exited });
}

/// Ends every running shell and everything it started, waiting at most
/// `grace` for a clean end before the survivors are killed. For the
/// application's quit; blocks the caller for up to `grace` plus a few
/// milliseconds, so it is not for the UI thread's normal operation.
pub fn end_all_sessions(grace: Duration) {
    let shells: Vec<(u32, Arc<AtomicBool>)> = {
        let mut live = LIVE.lock();
        live.retain(|l| !l.exited.load(Ordering::Acquire));
        live.iter().map(|l| (l.pid, l.exited.clone())).collect()
    };
    if shells.is_empty() {
        return;
    }
    // The descendants are listed first: once the shell is gone they are
    // reparented and no longer found through it.
    let mut doomed: Vec<u32> = Vec::new();
    for (pid, _) in &shells {
        doomed.extend(process::descendants(*pid));
    }
    for &pid in &doomed {
        process::hang_up_process(pid);
    }
    for (pid, _) in &shells {
        process::hang_up_process(*pid);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        let shells_gone = shells.iter().all(|(_, e)| e.load(Ordering::Acquire));
        if shells_gone && !doomed.iter().any(|&p| process::is_alive(p)) {
            return;
        }
        std::thread::sleep(POLL);
    }
    for &pid in &doomed {
        if process::is_alive(pid) {
            process::kill_process(pid);
        }
    }
    for (pid, exited) in &shells {
        if !exited.load(Ordering::Acquire) {
            process::kill_process(*pid);
        }
    }
}

/// Puts the shell into the job that ends with the application.
#[cfg(windows)]
pub(crate) fn join_job(process: windows_sys::Win32::Foundation::HANDLE) {
    job::assign(process);
}

#[cfg(windows)]
mod job {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// The job handle, never closed: it closes when the application ends,
    /// which kills what is in the job.
    struct Job(HANDLE);

    // SAFETY: a job handle may be used from any thread.
    unsafe impl Send for Job {}
    // SAFETY: as above.
    unsafe impl Sync for Job {}

    static JOB: OnceLock<Option<Job>> = OnceLock::new();

    fn job() -> Option<HANDLE> {
        JOB.get_or_init(|| {
            // SAFETY: plain calls with a zeroed information block of the
            // size the class expects.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                (ok != 0).then_some(Job(job))
            }
        })
        .as_ref()
        .map(|j| j.0)
    }

    pub(super) fn assign(process: HANDLE) {
        if let Some(job) = job() {
            // Fails when the application's own job forbids nesting: the
            // explicit end on quit still works then.
            // SAFETY: both handles are valid.
            unsafe { AssignProcessToJobObject(job, process) };
        }
    }
}
