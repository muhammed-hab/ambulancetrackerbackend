use std::time::Duration;
use sqlx::postgres::types::PgInterval;

/// Converts an interval to a duration
///
/// # Panics
///
/// If the interval is longer than 6 hours
pub fn convert_interval(interval: PgInterval) -> Duration {
	const SIX_HOURS_MICROSECONDS: i64 = 6 * 60 * 60 * 1000 * 1000;

	assert_eq!(interval.days, 0, "cannot be longer than 6 hours");
	assert_eq!(interval.months, 0, "cannot be longer than 6 hours");
	assert!(interval.microseconds >= 0 && interval.microseconds <= SIX_HOURS_MICROSECONDS, "cannot be longer than 6 hours");

	Duration::from_micros(interval.microseconds as u64)
}
