use super::*;

#[test]
fn test_parse_schedule_every_minutes() {
    assert_eq!(
        parse_schedule_to_cron("every 5 minutes").unwrap(),
        "*/5 * * * *"
    );
    assert_eq!(
        parse_schedule_to_cron("every 1 minute").unwrap(),
        "* * * * *"
    );
    assert_eq!(parse_schedule_to_cron("every minute").unwrap(), "* * * * *");
    assert_eq!(
        parse_schedule_to_cron("every 30 minutes").unwrap(),
        "*/30 * * * *"
    );
}

#[test]
fn test_parse_schedule_every_hours() {
    assert_eq!(parse_schedule_to_cron("every hour").unwrap(), "0 * * * *");
    assert_eq!(parse_schedule_to_cron("every 1 hour").unwrap(), "0 * * * *");
    assert_eq!(
        parse_schedule_to_cron("every 2 hours").unwrap(),
        "0 */2 * * *"
    );
}

#[test]
fn test_parse_schedule_daily() {
    assert_eq!(parse_schedule_to_cron("daily at 9am").unwrap(), "0 9 * * *");
    assert_eq!(
        parse_schedule_to_cron("daily at 6pm").unwrap(),
        "0 18 * * *"
    );
    assert_eq!(
        parse_schedule_to_cron("daily at 12am").unwrap(),
        "0 0 * * *"
    );
    assert_eq!(
        parse_schedule_to_cron("daily at 12pm").unwrap(),
        "0 12 * * *"
    );
}

#[test]
fn test_parse_schedule_weekdays() {
    assert_eq!(
        parse_schedule_to_cron("weekdays at 9am").unwrap(),
        "0 9 * * 1-5"
    );
    assert_eq!(
        parse_schedule_to_cron("weekends at 10am").unwrap(),
        "0 10 * * 0,6"
    );
}

#[test]
fn test_parse_schedule_shorthand() {
    assert_eq!(parse_schedule_to_cron("hourly").unwrap(), "0 * * * *");
    assert_eq!(parse_schedule_to_cron("daily").unwrap(), "0 0 * * *");
    assert_eq!(parse_schedule_to_cron("weekly").unwrap(), "0 0 * * 0");
    assert_eq!(parse_schedule_to_cron("monthly").unwrap(), "0 0 1 * *");
}

#[test]
fn test_parse_schedule_cron_passthrough() {
    assert_eq!(
        parse_schedule_to_cron("0 */5 * * *").unwrap(),
        "0 */5 * * *"
    );
    assert_eq!(
        parse_schedule_to_cron("30 9 * * 1-5").unwrap(),
        "30 9 * * 1-5"
    );
}

#[test]
fn test_parse_schedule_invalid() {
    assert!(parse_schedule_to_cron("whenever I feel like it").is_err());
    assert!(parse_schedule_to_cron("every 0 minutes").is_err());
}
