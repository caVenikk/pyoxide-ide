//! The pseudo-console check on Windows (ADR 0014): ConPTY exists since
//! Windows 10 1809 (build 17763). `alacritty_terminal` asserts when it
//! cannot create one, so the version is checked before a session starts.

/// The first Windows build with ConPTY.
pub const MIN_CONPTY_BUILD: u32 = 17763;

/// `Some(build)` when this Windows is too old for ConPTY; always `None`
/// elsewhere.
pub fn unsupported_windows_build() -> Option<u32> {
    #[cfg(windows)]
    {
        let build = windows_build();
        (build != 0 && build < MIN_CONPTY_BUILD).then_some(build)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The Windows build number, or 0 when it cannot be read. `RtlGetVersion`
/// reports the real version (`GetVersionEx` depends on the manifest).
#[cfg(windows)]
pub fn windows_build() -> u32 {
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;

    // SAFETY: a zeroed OSVERSIONINFOW with its size set is what the call
    // expects.
    unsafe {
        let mut info: OSVERSIONINFOW = std::mem::zeroed();
        info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        if RtlGetVersion(&mut info) == 0 {
            info.dwBuildNumber
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn this_system_has_a_pty() {
        assert_eq!(super::unsupported_windows_build(), None);
        #[cfg(windows)]
        assert!(super::windows_build() >= super::MIN_CONPTY_BUILD);
    }
}
