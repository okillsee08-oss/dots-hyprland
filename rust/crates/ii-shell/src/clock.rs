use std::time::{Duration, SystemTime, UNIX_EPOCH};

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The upstream clock defaults to minute precision and switches to seconds
/// only for its explicit second-precision/locked-screen modes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClockPrecision {
    #[default]
    Minutes,
    Seconds,
}

impl ClockPrecision {
    const fn interval_seconds(self) -> u64 {
        match self {
            Self::Minutes => 60,
            Self::Seconds => 1,
        }
    }
}

/// Formatted local-time strings shared by every output panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockText {
    pub time: String,
    pub long_date: String,
}

impl Default for ClockText {
    fn default() -> Self {
        Self {
            time: "--:--".into(),
            long_date: String::new(),
        }
    }
}

/// A clock snapshot whose next refresh is aligned to the next wall-clock
/// minute (or second) boundary. It is driven by the shell's existing poll
/// loop, not by a timer thread or a periodic wakeup.
#[derive(Debug)]
pub struct ClockState {
    text: ClockText,
    precision: ClockPrecision,
    last_refresh: SystemTime,
    next_tick: SystemTime,
}

impl ClockState {
    pub fn new(now: SystemTime) -> Self {
        Self::with_precision(now, ClockPrecision::Minutes)
    }

    pub fn with_precision(now: SystemTime, precision: ClockPrecision) -> Self {
        Self {
            text: format_local_time(now, precision).unwrap_or_default(),
            precision,
            last_refresh: now,
            next_tick: next_boundary(now, precision),
        }
    }

    pub fn text(&self) -> &ClockText {
        &self.text
    }

    /// Refresh after the next clock boundary or after a backwards wall-clock
    /// adjustment. Returns true only when visible clock text changed.
    pub fn refresh(&mut self, now: SystemTime) -> bool {
        if now < self.last_refresh || now >= self.next_tick {
            let next_text =
                format_local_time(now, self.precision).unwrap_or_else(|| self.text.clone());
            self.last_refresh = now;
            self.next_tick = next_boundary(now, self.precision);
            if next_text != self.text {
                self.text = next_text;
                return true;
            }
        }
        false
    }

    /// Timeout for poll(2), rounded up so a fractional millisecond cannot
    /// cause a premature clock wakeup.
    pub fn poll_timeout_ms(&self, now: SystemTime) -> i32 {
        let remaining = self.next_tick.duration_since(now).unwrap_or(Duration::ZERO);
        let millis = remaining.as_nanos().saturating_add(999_999) / 1_000_000;
        i32::try_from(millis).unwrap_or(i32::MAX)
    }
}

fn next_boundary(now: SystemTime, precision: ClockPrecision) -> SystemTime {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let interval = precision.interval_seconds();
    let next = seconds
        .saturating_sub(seconds % interval)
        .saturating_add(interval);
    UNIX_EPOCH + Duration::from_secs(next)
}

fn format_local_time(now: SystemTime, precision: ClockPrecision) -> Option<ClockText> {
    let seconds = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let timestamp = libc::time_t::try_from(seconds).ok()?;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    let result = unsafe { libc::localtime_r(&timestamp, local.as_mut_ptr()) };
    if result.is_null() {
        return None;
    }
    let local = unsafe { local.assume_init() };
    format_parts(&local, precision)
}

fn format_parts(local: &libc::tm, precision: ClockPrecision) -> Option<ClockText> {
    let hour = u32::try_from(local.tm_hour).ok()?;
    let minute = u32::try_from(local.tm_min).ok()?;
    let second = u32::try_from(local.tm_sec).ok()?;
    let weekday = WEEKDAYS.get(usize::try_from(local.tm_wday).ok()?)?;
    let day = u32::try_from(local.tm_mday).ok()?;
    let month = u32::try_from(local.tm_mon).ok()?.checked_add(1)?;

    if hour > 23 || minute > 59 || second > 60 || !(1..=31).contains(&day) || month > 12 {
        return None;
    }

    // Qt's upstream default "hh:mm" is a zero-padded 12-hour clock.
    let hour_12 = match hour % 12 {
        0 => 12,
        value => value,
    };
    let time = match precision {
        ClockPrecision::Minutes => format!("{hour_12:02}:{minute:02}"),
        ClockPrecision::Seconds => format!("{hour_12:02}:{minute:02}:{second:02}"),
    };

    Some(ClockText {
        time,
        long_date: format!("{weekday}, {day:02}/{month:02}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_time(hour: i32, minute: i32, second: i32, weekday: i32, day: i32, month: i32) -> libc::tm {
        let mut value: libc::tm = unsafe { std::mem::zeroed() };
        value.tm_hour = hour;
        value.tm_min = minute;
        value.tm_sec = second;
        value.tm_wday = weekday;
        value.tm_mday = day;
        value.tm_mon = month;
        value
    }

    #[test]
    fn minute_and_second_deadlines_align_to_the_next_boundary() {
        let now = UNIX_EPOCH + Duration::from_secs(125);
        assert_eq!(
            next_boundary(now, ClockPrecision::Minutes)
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            180
        );
        assert_eq!(
            next_boundary(now, ClockPrecision::Seconds)
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            126
        );
    }

    #[test]
    fn default_clock_matches_upstream_twelve_hour_format() {
        let midnight = local_time(0, 5, 0, 5, 13, 10);
        let text = format_parts(&midnight, ClockPrecision::Minutes).unwrap();
        assert_eq!(text.time, "12:05");
        assert_eq!(text.long_date, "Friday, 13/11");

        let evening = local_time(13, 9, 0, 5, 13, 10);
        let text = format_parts(&evening, ClockPrecision::Minutes).unwrap();
        assert_eq!(text.time, "01:09");
    }

    #[test]
    fn second_precision_includes_seconds_without_changing_date_format() {
        let local = local_time(23, 59, 7, 1, 2, 0);
        let text = format_parts(&local, ClockPrecision::Seconds).unwrap();
        assert_eq!(text.time, "11:59:07");
        assert_eq!(text.long_date, "Monday, 02/01");
    }

    #[test]
    fn invalid_local_time_components_are_rejected() {
        let local = local_time(24, 0, 0, 0, 1, 0);
        assert!(format_parts(&local, ClockPrecision::Minutes).is_none());
    }

    #[test]
    fn poll_timeout_rounds_fractional_milliseconds_up() {
        let now = UNIX_EPOCH + Duration::new(125, 1);
        let clock = ClockState::new(now);
        assert_eq!(clock.poll_timeout_ms(now), 55_000);
    }
}
