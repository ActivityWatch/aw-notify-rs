use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use jiff::{civil::DateTimeRound, RoundMode, ToSpan, Unit, Zoned, ZonedRound};

pub fn to_utc(time: &Zoned) -> Result<DateTime<Utc>> {
    let nanos = time.timestamp().as_nanosecond();
    DateTime::from_timestamp(
        nanos.div_euclid(1_000_000_000) as i64,
        nanos.rem_euclid(1_000_000_000) as u32,
    )
    .ok_or_else(|| anyhow!("Timestamp is out of range"))
}

/// Resolve 04:00 on each calendar date independently, preserving 23/25-hour days.
pub fn day_bounds(date: &Zoned) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let start = date
        .date()
        .at(crate::TIME_OFFSET.num_hours() as i8, 0, 0, 0);
    let timezone = date.time_zone();
    Ok((
        to_utc(&start.to_zoned(timezone.clone())?)?,
        to_utc(&start.tomorrow()?.to_zoned(timezone.clone())?)?,
    ))
}

/// Next civil hour: Jiff shifts gaps forward and chooses the earlier fold.
pub fn next_hour_delay(now: &Zoned) -> Result<std::time::Duration> {
    let next = now
        .datetime()
        .checked_add(1.hour())?
        .round(DateTimeRound::from(Unit::Hour).mode(RoundMode::Floor))?
        .to_zoned(now.time_zone().clone())?
        // A half-hour gap can resolve 02:00 to 02:30; keep check-ins on the hour.
        .round(ZonedRound::from(Unit::Hour).mode(RoundMode::Ceil))?;
    Ok(now.duration_until(&next).try_into()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn daily_boundaries_stay_at_four_across_dst() -> Result<()> {
        for (date, hours) in [("2026-03-07", 23), ("2026-10-31", 25), ("2026-01-01", 24)] {
            let now: Zoned = format!("{date}T12:00[America/New_York]").parse()?;
            let (start, end) = day_bounds(&now)?;
            assert_eq!(end - start, Duration::hours(hours));
            for boundary in [start, end] {
                let local = jiff::Timestamp::from_second(boundary.timestamp())?
                    .to_zoned(now.time_zone().clone());
                assert_eq!(local.hour(), 4);
            }
        }
        Ok(())
    }

    #[test]
    fn hourly_checkins_follow_jiff_transition_policy() -> Result<()> {
        for (input, minutes) in [
            ("2026-03-08T01:30-05:00[America/New_York]", 30),
            ("2026-11-01T00:30-04:00[America/New_York]", 30),
            ("2026-11-01T01:00-04:00[America/New_York]", 120),
            ("2026-11-01T01:30-05:00[America/New_York]", 30),
            ("2026-01-01T23:00[Asia/Kathmandu]", 60),
            ("2026-04-05T01:15[Australia/Lord_Howe]", 75),
            ("2026-10-04T01:45[Australia/Lord_Howe]", 45),
            ("2026-10-04T02:30[Australia/Lord_Howe]", 30),
        ] {
            assert_eq!(
                next_hour_delay(&input.parse()?)?,
                std::time::Duration::from_secs(minutes * 60)
            );
        }
        Ok(())
    }
}
