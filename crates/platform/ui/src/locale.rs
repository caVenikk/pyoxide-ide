//! Dates, times and numbers in the format of the system's region settings,
//! as the reference shows them (`DateFormatUtil.formatDateTime`: the
//! system's short date and short time; `StringUtil.formatFileSize`: the
//! locale's decimal separator).
//!
//! - macOS: `NSDateFormatter` with the short date and time styles and
//!   `NSLocale`'s decimal separator, so custom region formats apply.
//! - Windows: `GetDateFormatEx` (short date) and `GetTimeFormatEx` (no
//!   seconds) of the user's locale, joined by a space as the reference
//!   joins them, and the locale's decimal separator.
//! - Linux: the `LC_TIME` locale's `D_FMT` and `T_FMT` without seconds,
//!   the `LC_NUMERIC` locale's radix char.
//!
//! Blocking but cheap; call off the UI thread together with the I/O that
//! produced the values when there is some.

use std::time::SystemTime;

/// A point in time as the system's short date and short time
/// (`18.09.2026, 14:09` in a Russian region on macOS, `9/18/26, 2:09 PM`
/// in a US one).
pub fn date_time(time: SystemTime) -> String {
    platform::date_time(time).unwrap_or_else(|| fallback_date_time(time))
}

/// The decimal separator of the system's number format (`.` or `,`).
pub fn decimal_separator() -> char {
    platform::decimal_separator().unwrap_or('.')
}

/// A file size as the reference writes it (`StringUtilRt.formatFileSize`):
/// powers of 1000 with the units `B`, `kB`, `MB`, ..., at most two
/// fraction digits without trailing zeros, the locale's decimal separator
/// (`9,74 kB`).
pub fn file_size(bytes: u64) -> String {
    file_size_with(bytes, decimal_separator())
}

/// [`file_size`] with a given decimal separator.
pub fn file_size_with(bytes: u64, separator: char) -> String {
    const UNITS: [&str; 7] = ["B", "kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes == 0 {
        return "0 B".to_owned();
    }
    // `rankForFileSize`: the rank whose value rounds below 1000.
    let rank = ((((bytes as f64).log10() + 0.000_002_171_477_838_430_746_5) / 3.0) as usize)
        .min(UNITS.len() - 1);
    let value = bytes as f64 / 1000f64.powi(rank as i32);
    // `DecimalFormat("0.##")`: half-even rounding to two fraction digits.
    let hundredths = round_half_even(value * 100.0);
    let whole = (hundredths / 100.0).trunc();
    let fraction = (hundredths - whole * 100.0).round() as u32;
    let mut text = format!("{whole:.0}");
    if fraction > 0 {
        let digits = format!("{fraction:02}");
        text.push(separator);
        text.push_str(digits.trim_end_matches('0'));
    }
    format!("{text} {}", UNITS[rank])
}

fn round_half_even(x: f64) -> f64 {
    let floor = x.floor();
    let diff = x - floor;
    if (diff - 0.5).abs() < 1e-9 {
        if floor % 2.0 == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    } else {
        x.round()
    }
}

/// Without the system's format: ISO date and 24-hour time in UTC.
fn fallback_date_time(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{CStr, c_char};
    use std::time::SystemTime;

    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    /// `NSDateFormatterShortStyle`.
    const SHORT: u64 = 1;

    /// The text of an `NSString`.
    ///
    /// # Safety
    /// `string` is null or a live `NSString`.
    #[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
    unsafe fn text(string: *mut Object) -> Option<String> {
        if string.is_null() {
            return None;
        }
        // SAFETY: a live NSString; its UTF-8 buffer lives as long as the
        // autorelease pool around the caller.
        unsafe {
            let utf8: *const c_char = msg_send![string, UTF8String];
            (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
        }
    }

    /// Runs `f` inside an autorelease pool (the objects Foundation's class
    /// methods return are autoreleased; this may run on any thread).
    #[allow(unexpected_cfgs)]
    fn with_pool<T>(f: impl FnOnce() -> T) -> T {
        // SAFETY: NSAutoreleasePool is usable on any thread; drained on the
        // thread that made it.
        unsafe {
            let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
            let out = f();
            let _: () = msg_send![pool, drain];
            out
        }
    }

    #[allow(unexpected_cfgs)]
    pub fn date_time(time: SystemTime) -> Option<String> {
        let secs = match time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64(),
            Err(e) => -e.duration().as_secs_f64(),
        };
        with_pool(|| {
            // SAFETY: Foundation class methods with plain arguments; the
            // results are autoreleased into the pool.
            unsafe {
                let date: *mut Object =
                    msg_send![class!(NSDate), dateWithTimeIntervalSince1970: secs];
                if date.is_null() {
                    return None;
                }
                let string: *mut Object = msg_send![
                    class!(NSDateFormatter),
                    localizedStringFromDate: date
                    dateStyle: SHORT
                    timeStyle: SHORT
                ];
                text(string)
            }
        })
    }

    #[allow(unexpected_cfgs)]
    pub fn decimal_separator() -> Option<char> {
        with_pool(|| {
            // SAFETY: Foundation calls without arguments; autoreleased
            // results.
            unsafe {
                let locale: *mut Object = msg_send![class!(NSLocale), currentLocale];
                if locale.is_null() {
                    return None;
                }
                let separator: *mut Object = msg_send![locale, decimalSeparator];
                text(separator)?.chars().next()
            }
        })
    }
}

#[cfg(windows)]
mod platform {
    use std::time::SystemTime;

    use chrono::{Datelike, Timelike};

    #[repr(C)]
    struct SystemTimeFields {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    const DATE_SHORTDATE: u32 = 0x1;
    const TIME_NOSECONDS: u32 = 0x2;
    const LOCALE_SDECIMAL: u32 = 0xE;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDateFormatEx(
            locale: *const u16,
            flags: u32,
            date: *const SystemTimeFields,
            format: *const u16,
            out: *mut u16,
            len: i32,
            calendar: *const u16,
        ) -> i32;
        fn GetTimeFormatEx(
            locale: *const u16,
            flags: u32,
            time: *const SystemTimeFields,
            format: *const u16,
            out: *mut u16,
            len: i32,
        ) -> i32;
        fn GetLocaleInfoEx(locale: *const u16, kind: u32, out: *mut u16, len: i32) -> i32;
    }

    fn wide(buffer: &[u16], written: i32) -> Option<String> {
        // `written` counts the terminating zero.
        let len = usize::try_from(written).ok()?.checked_sub(1)?;
        String::from_utf16(buffer.get(..len)?).ok()
    }

    pub fn date_time(time: SystemTime) -> Option<String> {
        let local: chrono::DateTime<chrono::Local> = time.into();
        let fields = SystemTimeFields {
            year: u16::try_from(local.year()).ok()?,
            month: local.month() as u16,
            day_of_week: local.weekday().num_days_from_sunday() as u16,
            day: local.day() as u16,
            hour: local.hour() as u16,
            minute: local.minute() as u16,
            second: local.second() as u16,
            milliseconds: 0,
        };
        let mut date = [0u16; 128];
        let mut clock = [0u16; 128];
        // SAFETY: valid buffers with their lengths; a null locale is the
        // user's default.
        let (d, t) = unsafe {
            (
                GetDateFormatEx(
                    std::ptr::null(),
                    DATE_SHORTDATE,
                    &fields,
                    std::ptr::null(),
                    date.as_mut_ptr(),
                    date.len() as i32,
                    std::ptr::null(),
                ),
                GetTimeFormatEx(
                    std::ptr::null(),
                    TIME_NOSECONDS,
                    &fields,
                    std::ptr::null(),
                    clock.as_mut_ptr(),
                    clock.len() as i32,
                ),
            )
        };
        Some(format!("{} {}", wide(&date, d)?, wide(&clock, t)?))
    }

    pub fn decimal_separator() -> Option<char> {
        let mut buffer = [0u16; 8];
        // SAFETY: a valid buffer with its length; null is the user's
        // default locale.
        let n = unsafe {
            GetLocaleInfoEx(
                std::ptr::null(),
                LOCALE_SDECIMAL,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        wide(&buffer, n)?.chars().next()
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::CStr;
    use std::time::SystemTime;

    /// An item of the environment's locale (`LC_TIME`, `LC_NUMERIC` from
    /// `LC_ALL` / `LC_*` / `LANG`), `None` when the locale is not
    /// installed.
    fn item(mask: libc::c_int, item: libc::nl_item) -> Option<String> {
        // SAFETY: newlocale with an empty name reads the environment; the
        // locale object is freed after its string is copied.
        unsafe {
            let locale = libc::newlocale(mask, c"".as_ptr(), std::ptr::null_mut());
            if locale.is_null() {
                return None;
            }
            let value = libc::nl_langinfo_l(item, locale);
            let text =
                (!value.is_null()).then(|| CStr::from_ptr(value).to_string_lossy().into_owned());
            libc::freelocale(locale);
            text
        }
    }

    /// A `T_FMT` without its seconds.
    pub(super) fn short_time(format: &str) -> String {
        format
            .replace("%T", "%H:%M")
            .replace("%r", "%I:%M %p")
            .replace(":%S", "")
            .replace(".%S", "")
    }

    pub fn date_time(time: SystemTime) -> Option<String> {
        use chrono::format::{Item, StrftimeItems};
        let date = item(libc::LC_TIME_MASK, libc::D_FMT)?;
        let clock = short_time(&item(libc::LC_TIME_MASK, libc::T_FMT)?);
        let format = format!("{date} {clock}");
        if StrftimeItems::new(&format).any(|i| matches!(i, Item::Error)) {
            return None;
        }
        let local: chrono::DateTime<chrono::Local> = time.into();
        Some(local.format(&format).to_string())
    }

    pub fn decimal_separator() -> Option<char> {
        item(libc::LC_NUMERIC_MASK, libc::RADIXCHAR)?.chars().next()
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    use std::time::SystemTime;

    pub fn date_time(_: SystemTime) -> Option<String> {
        None
    }

    pub fn decimal_separator() -> Option<char> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn file_sizes_as_the_reference_writes_them() {
        assert_eq!(file_size_with(0, ','), "0 B");
        assert_eq!(file_size_with(999, ','), "999 B");
        assert_eq!(file_size_with(1000, ','), "1 kB");
        assert_eq!(file_size_with(9740, ','), "9,74 kB");
        assert_eq!(file_size_with(9745, '.'), "9.74 kB");
        assert_eq!(file_size_with(9_999_999, '.'), "10 MB");
        assert_eq!(file_size_with(1_500_000, '.'), "1.5 MB");
        assert_eq!(file_size_with(999_995, '.'), "1 MB");
    }

    #[test]
    fn dates_have_the_day_and_the_minute() {
        // 2026-09-18 12:09:00 UTC.
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_789_733_340);
        let text = date_time(time);
        assert!(text.contains("09") || text.contains('9'), "{text}");
        assert!(!text.is_empty());
        assert_eq!(fallback_date_time(time), "2026-09-18 12:09");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn times_without_seconds() {
        assert_eq!(platform::short_time("%T"), "%H:%M");
        assert_eq!(platform::short_time("%r"), "%I:%M %p");
        assert_eq!(platform::short_time("%H:%M:%S"), "%H:%M");
    }
}
