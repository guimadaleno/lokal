//! When a message was sent, as the screen shows it (#46): the computer's
//! local time, hours and minutes, faint at the right edge. A time shows
//! only when its minute isn't the last one shown, so a quick exchange
//! carries one and the rest read as the same minute as the time above. The
//! date joins it on the first time of a day other than the last one shown,
//! so a session left open overnight says so.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use unicode_width::UnicodeWidthStr;

use crate::text;

/// The minute of the last time shown: its year, day in it, hour and minute.
type Minute = (i32, i32, i32, i32);

static LAST: Mutex<Option<Minute>> = Mutex::new(None);

/// `times = off` in the settings: no time shows.
static OFF: AtomicBool = AtomicBool::new(false);

/// No time shows from now on, as the settings ask.
pub fn switch_off() {
    OFF.store(true, Ordering::Relaxed);
}

/// The screen was cleared, and the last time shown with it: the next one
/// shows whatever its minute. Its day stays, so a new day still shows its
/// date.
pub fn forget() {
    let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
    *last = forgotten(*last);
}

/// The last minute shown, with only its day left.
fn forgotten(last: Option<Minute>) -> Option<Minute> {
    last.map(|(year, day, ..)| (year, day, -1, -1))
}

/// The local time at `at`, broken down, or None where the clock can't say.
pub fn local(at: SystemTime) -> Option<libc::tm> {
    let seconds = libc::time_t::try_from(at.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    // SAFETY: localtime_r fills the tm it's given, which is plain data.
    unsafe {
        let mut local: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&seconds, &mut local).is_null()).then_some(local)
    }
}

/// `at` as the screen shows it, "14:32" or "Nov 15 09:12", when its minute
/// isn't the last one shown and `fits` says its width has room. Only a time
/// that shows counts as the last one shown, so a time or a date isn't lost
/// to a line without room.
pub fn shown(at: SystemTime, fits: impl Fn(usize) -> bool) -> Option<String> {
    if OFF.load(Ordering::Relaxed) {
        return None;
    }
    let local = local(at)?;
    let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
    let (time, minute) = written(&local, *last)?;
    fits(time.width()).then(|| {
        *last = Some(minute);
        time
    })
}

/// The time, with its date when its day isn't `last`'s, and its minute;
/// None when it's `last`'s minute.
fn written(local: &libc::tm, last: Option<Minute>) -> Option<(String, Minute)> {
    let minute = (local.tm_year, local.tm_yday, local.tm_hour, local.tm_min);
    if last == Some(minute) {
        return None;
    }
    let date = last
        .is_some_and(|(year, day, ..)| (year, day) != (minute.0, minute.1))
        .then_some((local.tm_mon, local.tm_mday));
    Some((text::clock(local.tm_hour, local.tm_min, date), minute))
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

    /// The times a run of moments shows, from a fresh screen.
    fn run(moments: &[libc::tm]) -> Vec<Option<String>> {
        let mut last = None;
        moments
            .iter()
            .map(|moment| {
                written(moment, last).map(|(time, minute)| {
                    last = Some(minute);
                    time
                })
            })
            .collect()
    }

    #[test]
    fn a_time_shows_only_when_its_minute_changed() {
        let first = at(126, 317, 10, 14, 10, 40);
        let later = at(126, 317, 10, 14, 10, 47);
        let shown = run(&[first, first, first, later, later, first]);
        let expected = ["10:40", "", "", "10:47", "", "10:40"];
        let expected: Vec<Option<String>> = expected
            .iter()
            .map(|time| (!time.is_empty()).then(|| time.to_string()))
            .collect();
        assert_eq!(shown, expected);
    }

    #[test]
    fn a_cleared_screen_shows_the_next_time_and_a_new_day_its_date() {
        let evening = at(126, 317, 10, 14, 23, 50);
        let last = written(&evening, None).map(|(_, minute)| minute);
        // The same minute again, after a clear: it shows.
        assert_eq!(
            written(&evening, forgotten(last)).map(|(time, _)| time),
            Some("23:50".into())
        );
        let morning = at(126, 318, 10, 15, 9, 5);
        assert_eq!(
            written(&morning, forgotten(last)).map(|(time, _)| time),
            Some("Nov 15 09:05".into())
        );
    }

    #[test]
    fn the_date_shows_only_when_the_day_changed() {
        let morning = at(126, 317, 10, 14, 9, 5);
        // The first time a session shows: today, so no date.
        assert_eq!(run(&[morning]), [Some("09:05".into())]);
        let next = at(126, 318, 10, 15, 0, 2);
        assert_eq!(run(&[morning, next])[1].as_deref(), Some("Nov 15 00:02"));
        // A year later on the same day of the year is another day.
        let next_year = at(127, 317, 10, 14, 9, 5);
        assert_eq!(
            run(&[morning, next_year])[1].as_deref(),
            Some("Nov 14 09:05")
        );
    }

    #[test]
    fn the_clock_reads_the_local_time() {
        // A minute no other test shows, so the shared last one isn't it.
        let moment = UNIX_EPOCH + std::time::Duration::from_secs(60 * 1_000_003);
        let local = local(moment).unwrap();
        assert!((0..24).contains(&local.tm_hour) && (0..60).contains(&local.tm_min));
        assert_eq!(shown(moment, |_| false), None);
        let time = shown(moment, |_| true).unwrap();
        assert!(time.ends_with(&format!("{:02}:{:02}", local.tm_hour, local.tm_min)));
    }
}
