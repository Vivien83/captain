use crate::cli_captain_home;

#[derive(Debug, Clone)]
pub(crate) struct CliLogEvent {
    pub(crate) id: i64,
    pub(crate) session_id: String,
    pub(crate) ts: i64,
    pub(crate) event_type: String,
    pub(crate) payload: serde_json::Value,
}

pub(crate) fn read_session_events(
    after_id: Option<i64>,
    since_ms: Option<i64>,
    scan_limit: usize,
) -> Result<Vec<CliLogEvent>, rusqlite::Error> {
    let db_path = cli_captain_home().join("data").join("captain.db");
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let limit = scan_limit.clamp(1, 10_000) as i64;
    let since = since_ms.unwrap_or(i64::MIN);

    let mut events = if let Some(after_id) = after_id {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, ts, event_type, payload
             FROM sessions_events
             WHERE id > ?1 AND ts >= ?2
             ORDER BY id ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![after_id, since, limit],
            row_to_cli_log_event,
        )?;
        collect_log_events(rows)?
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, ts, event_type, payload
             FROM sessions_events
             WHERE ts >= ?1
             ORDER BY ts DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![since, limit], row_to_cli_log_event)?;
        let mut rows = collect_log_events(rows)?;
        rows.reverse();
        rows
    };
    events.sort_by_key(|ev| (ev.ts, ev.id));
    Ok(events)
}

fn row_to_cli_log_event(row: &rusqlite::Row<'_>) -> Result<CliLogEvent, rusqlite::Error> {
    let payload_str: String = row.get(4)?;
    let payload = serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
    Ok(CliLogEvent {
        id: row.get(0)?,
        session_id: row.get(1)?,
        ts: row.get(2)?,
        event_type: row.get(3)?,
        payload,
    })
}

fn collect_log_events<I>(rows: I) -> Result<Vec<CliLogEvent>, rusqlite::Error>
where
    I: IntoIterator<Item = Result<CliLogEvent, rusqlite::Error>>,
{
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn summarize_log_payload(event: &CliLogEvent) -> String {
    match event.event_type.as_str() {
        "tool_use_start" => {
            let name = event.payload["name"].as_str().unwrap_or("?");
            format!("start {name}")
        }
        "tool_use_end" => {
            let name = event.payload["name"].as_str().unwrap_or("?");
            let input = event
                .payload
                .get("input")
                .map(|v| clip_log_text(&v.to_string(), 160))
                .unwrap_or_default();
            format!("end {name} {input}")
        }
        "tool_execution_result" => {
            let name = event.payload["name"].as_str().unwrap_or("?");
            let status = if event.payload["is_error"].as_bool().unwrap_or(false) {
                "error"
            } else {
                "ok"
            };
            let preview = event.payload["result_preview"].as_str().unwrap_or_default();
            format!("{status} {name} {}", clip_log_text(preview, 180))
        }
        "phase_change" => {
            let phase = event.payload["phase"].as_str().unwrap_or("?");
            let detail = event.payload["detail"].as_str().unwrap_or_default();
            if detail.is_empty() {
                phase.to_string()
            } else {
                format!("{phase}: {detail}")
            }
        }
        _ => clip_log_text(&event.payload.to_string(), 220),
    }
}

fn clip_log_text(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    out
}

pub(crate) fn parse_log_since_ms(input: &str) -> Result<i64, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Empty --since value".to_string());
    }
    if let Some((value, unit)) = s.split_at_checked(s.len().saturating_sub(1)) {
        if matches!(unit, "s" | "m" | "h" | "d") {
            let n: i64 = value
                .parse()
                .map_err(|_| format!("Invalid --since duration: {input}"))?;
            let secs = match unit {
                "s" => n,
                "m" => n * 60,
                "h" => n * 60 * 60,
                "d" => n * 24 * 60 * 60,
                _ => unreachable!(),
            };
            return Ok(now_unix_ms().saturating_sub(secs.saturating_mul(1000)));
        }
    }
    parse_utc_timestamp_ms(s).ok_or_else(|| format!("Invalid --since timestamp: {input}"))
}

pub(super) fn parse_log_line_timestamp_ms(line: &str) -> Option<i64> {
    if line.len() < 19 {
        return None;
    }
    parse_utc_timestamp_ms(line)
}

pub(crate) fn parse_utc_timestamp_ms(input: &str) -> Option<i64> {
    let s = input.trim_start();
    if s.len() < 19 {
        return None;
    }
    let date_time = s.get(0..19)?;
    let bytes = date_time.as_bytes();
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T') | Some(b' '))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let year: i32 = date_time.get(0..4)?.parse().ok()?;
    let month: u32 = date_time.get(5..7)?.parse().ok()?;
    let day: u32 = date_time.get(8..10)?.parse().ok()?;
    let hour: i64 = date_time.get(11..13)?.parse().ok()?;
    let minute: i64 = date_time.get(14..16)?.parse().ok()?;
    let second: i64 = date_time.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    let mut millis = 0_i64;
    if s.as_bytes().get(19) == Some(&b'.') {
        let frac: String = s
            .get(20..)?
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(3)
            .collect();
        if !frac.is_empty() {
            let padded = format!("{frac:0<3}");
            millis = padded.parse().ok()?;
        }
    }
    let days = days_from_civil(year, month, day)?;
    Some(
        days.saturating_mul(86_400_000)
            .saturating_add(hour * 3_600_000)
            .saturating_add(minute * 60_000)
            .saturating_add(second * 1000)
            .saturating_add(millis),
    )
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut y = year as i64;
    let m = month as i64;
    let d = day as i64;
    y -= (m <= 2) as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 + doy - yoe / 100;
    Some(era * 146_097 + doe - 719_468)
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + (m <= 2) as i64, m as u32, d as u32)
}

pub(crate) fn format_unix_ms_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = sod / 3_600;
    let minute = (sod % 3_600) / 60;
    let second = sod % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_timestamp_roundtrip_utc_millis() {
        let ts = parse_utc_timestamp_ms("2026-05-05T20:40:45.550839Z").unwrap();
        assert_eq!(format_unix_ms_utc(ts), "2026-05-05T20:40:45.550Z");
    }

    #[test]
    fn log_payload_summary_handles_tool_errors() {
        let event = CliLogEvent {
            id: 1,
            session_id: "agent-1".to_string(),
            ts: 0,
            event_type: "tool_execution_result".to_string(),
            payload: serde_json::json!({
                "name": "web_fetch",
                "is_error": true,
                "result_preview": "network failed"
            }),
        };

        assert_eq!(
            summarize_log_payload(&event),
            "error web_fetch network failed"
        );
    }
}
