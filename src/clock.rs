//! When a message was sent, as the screen shows it (#46): the computer's
//! local time, hours and minutes, faint at the right edge. The date joins
//! it only on the first time of a day other than the last one shown, so a
//! session left open overnight says so and a busy one stays quiet.

use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use unicode_width::UnicodeWidthStr;

use crate::text;

/// The day of the last time shown: the year, and the day in it.
static LAST_DAY: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// The local time at `at`, broken down, or None where the clock can't say.
pub fn local(at: SystemTime) -> Option<libc::tm> {
    let seconds = libc::time_t::try_from(at.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    // SAFETY: localtime_r fills the tm it's given, which is plain data.
    unsafe {
        let mut local: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&seconds, &mut local).is_null()).then_some(local)
    }
}

/// `at` as the screen shows it, "14:32" or "Nov 15 09:12", when `fits`
/// says its width has room. Only a time that shows counts as the last one
/// shown, so a new day's date isn't lost to a line without room.
pub fn shown(at: SystemTime, fits: impl Fn(usize) -> bool) -> Option<String> {
    let local = local(at)?;
    let mut last = LAST_DAY.lock().unwrap_or_else(PoisonError::into_inner);
    let (time, day) = written(&local, *last);
    fits(time.width()).then(|| {
        *last = Some(day);
        time
    })
}

/// The time, with its date when its day isn't `last`, and its day.
fn written(local: &libc::tm, last: Option<(i32, i32)>) -> (String, (i32, i32)) {
    let day = (local.tm_year, local.tm_yday);
    let date = last
        .is_some_and(|last| last != day)
        .then_some((local.tm_mon, local.tm_mday));
    (text::clock(local.tm_hour, local.tm_min, date), day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, yday: i32, mon: i32, mday: i32, hour: i32, min: i32) -> libc::tm {
        // SAFETY: tm is plain data, and all zeros is a valid one.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        (tm.tm_year, tm.tm_yday, tm.tm_mon, tm.tm_mday) = (year, yday, mon, mday);
        (tm.tm_hour, tm.tm_min) = (hour, min);
        tm
    }

    #[test]
    fn the_date_shows_only_when_the_day_changed() {
        let morning = at(126, 317, 10, 14, 9, 5);
        // The first time a session shows: today, so no date.
        assert_eq!(written(&morning, None), ("09:05".into(), (126, 317)));
        assert_eq!(written(&morning, Some((126, 317))).0, "09:05");
        let next = at(126, 318, 10, 15, 0, 2);
        assert_eq!(written(&next, Some((126, 317))).0, "Nov 15 00:02");
        // A year later on the same day of the year is another day.
        assert_eq!(written(&morning, Some((125, 317))).0, "Nov 14 09:05");
    }

    #[test]
    fn the_clock_reads_the_local_time() {
        let now = SystemTime::now();
        let local = local(now).unwrap();
        assert!((0..24).contains(&local.tm_hour) && (0..60).contains(&local.tm_min));
        let time = shown(now, |_| true).unwrap();
        assert!(time.ends_with(&format!("{:02}:{:02}", local.tm_hour, local.tm_min)));
        assert_eq!(shown(now, |_| false), None);
    }
}
