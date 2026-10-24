use crate::{domain::*, safety};

pub struct Command {
    pub args: Vec<Vec<u8>>,
    pub kind: StatementKind,
    pub source: String,
}

// A newline ends a command outside quotes. Semicolons are ordinary argument
// bytes, never SQL separators. The same parser runs at review and execution.
pub fn parse(source: &str) -> AppResult<Vec<Command>> {
    if source.len() > 1024 * 1024 {
        return Err(AppError::invalid("Redis batches must fit within 1 MiB"));
    }
    let bytes = source.as_bytes();
    let mut commands = Vec::new();
    let (mut args, mut token) = (Vec::new(), Vec::new());
    let (mut quote, mut started, mut start, mut i) = (None, false, 0, 0);
    while i <= bytes.len() {
        let byte = bytes.get(i).copied();
        if byte.is_none() && quote.is_some() {
            return Err(AppError::invalid("Close the quoted Redis argument"));
        }
        if byte == Some(b'\\') {
            started = true;
            i += 1;
            let escaped = *bytes
                .get(i)
                .ok_or_else(|| AppError::invalid("Incomplete Redis escape"))?;
            token.push(match escaped {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'\\' => b'\\',
                b'"' => b'"',
                b'\'' => b'\'',
                b'x' => {
                    let digits = bytes
                        .get(i + 1..i + 3)
                        .ok_or_else(|| AppError::invalid("Use two hexadecimal digits after \\x"))?;
                    let value = std::str::from_utf8(digits)
                        .ok()
                        .and_then(|v| u8::from_str_radix(v, 16).ok())
                        .ok_or_else(|| AppError::invalid("Invalid hexadecimal escape"))?;
                    i += 2;
                    value
                }
                _ => {
                    return Err(AppError::invalid(
                        "Supported Redis escapes: \\n, \\r, \\t, \\xHH, quotes and backslash",
                    ));
                }
            });
        } else if let Some(q) = quote {
            if byte == Some(q) {
                quote = None;
            } else {
                token.push(byte.unwrap());
            }
        } else if matches!(byte, Some(b'"' | b'\'')) {
            started = true;
            quote = byte;
        } else if byte.is_none() || byte.is_some_and(|v| v.is_ascii_whitespace()) {
            if started {
                args.push(std::mem::take(&mut token));
                started = false;
            }
            if byte.is_none() || matches!(byte, Some(b'\n' | b'\r')) {
                if !args.is_empty() {
                    let kind = classify(&args)?;
                    commands.push(Command {
                        args: std::mem::take(&mut args),
                        kind,
                        source: source[start..i].trim().into(),
                    });
                    if commands.len() > 32 {
                        return Err(AppError::invalid(
                            "Run at most 32 Redis commands in a batch",
                        ));
                    }
                }
                start = i + 1;
            }
        } else {
            started = true;
            token.push(byte.unwrap());
        }
        if token.len() > 256 * 1024 || args.len() > 128 {
            return Err(AppError::invalid(
                "A Redis argument is limited to 256 KiB and a command to 128 arguments",
            ));
        }
        i += 1;
    }
    if commands.is_empty() {
        return Err(AppError::invalid("Enter a Redis command"));
    }
    Ok(commands)
}

pub fn prepare(
    session: &str,
    profile: &ConnectionProfile,
    source: &str,
) -> AppResult<PreparedPlan> {
    let commands = parse(source)?;
    let writes = commands.iter().any(|c| c.kind == StatementKind::Write);
    let mut plan = safety::plan_for_statements(
        session,
        profile,
        commands
            .into_iter()
            .map(|c| StatementPlan {
                sql: c.source,
                kind: c.kind,
                missing_where: false,
            })
            .collect(),
    )?;
    if writes {
        plan.requires_approval = true;
        plan.warnings.push("Redis commands execute individually, without automatic rollback or write retries. Values may have changed since inspection. SET without KEEPTTL clears an existing expiry.".into());
    }
    Ok(plan)
}

fn classify(args: &[Vec<u8>]) -> AppResult<StatementKind> {
    let name = std::str::from_utf8(&args[0])
        .map_err(|_| AppError::invalid("Redis command names must be ASCII"))?
        .to_ascii_uppercase();
    let n = args.len() - 1;
    let exact = |expected| {
        if n == expected {
            Ok(())
        } else {
            Err(AppError::invalid("Unexpected Redis argument count"))
        }
    };
    let range = |min, max| {
        if (min..=max).contains(&n) {
            Ok(())
        } else {
            Err(AppError::invalid("Unexpected Redis argument count"))
        }
    };
    let integer = |at: usize| {
        std::str::from_utf8(&args[at])
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(|| AppError::invalid("Expected a bounded Redis integer"))
    };
    let bounded_range = |start, end| -> AppResult<()> {
        let a = integer(start)?;
        let b = integer(end)?;
        if a < 0 || b < a || b - a >= 200 {
            Err(AppError::invalid(
                "Use a nonnegative range of at most 200 elements",
            ))
        } else {
            Ok(())
        }
    };
    let read = match name.as_str() {
        "PING" => {
            range(0, 1)?;
            true
        }
        "DBSIZE" => {
            exact(0)?;
            true
        }
        "GET" | "TYPE" | "TTL" | "PTTL" | "STRLEN" | "HLEN" | "LLEN" | "SCARD" | "ZCARD" => {
            exact(1)?;
            true
        }
        "HGET" => {
            exact(2)?;
            true
        }
        "HMGET" => {
            range(2, 101)?;
            true
        }
        "MGET" | "EXISTS" => {
            range(1, 100)?;
            true
        }
        "GETRANGE" => {
            exact(3)?;
            let a = integer(2)?;
            let b = integer(3)?;
            if a < 0 || b < a || b - a >= 65536 {
                return Err(AppError::invalid(
                    "Read at most 64 KiB using a nonnegative byte range",
                ));
            }
            true
        }
        "LRANGE" => {
            exact(3)?;
            bounded_range(2, 3)?;
            true
        }
        "ZRANGE" => {
            range(3, 4)?;
            bounded_range(2, 3)?;
            if n == 4 && !args[4].eq_ignore_ascii_case(b"WITHSCORES") {
                return Err(AppError::invalid(
                    "Only WITHSCORES is supported after a ZRANGE",
                ));
            }
            true
        }
        "SCAN" | "HSCAN" | "SSCAN" | "ZSCAN" => {
            let cursor = if name == "SCAN" { 1 } else { 2 };
            range(cursor, cursor + 6)?;
            if std::str::from_utf8(&args[cursor])
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .is_none()
            {
                return Err(AppError::invalid(
                    "Use the cursor returned by the preceding scan",
                ));
            }
            let mut seen = std::collections::HashSet::new();
            for pair in args[cursor + 1..].chunks(2) {
                if pair.len() != 2 {
                    return Err(AppError::invalid("SCAN options need a value"));
                }
                let option = String::from_utf8_lossy(&pair[0]).to_ascii_uppercase();
                if !seen.insert(option.clone()) {
                    return Err(AppError::invalid("Do not repeat SCAN options"));
                }
                match option.as_str() {
                    "MATCH" => {}
                    "COUNT" => {
                        if std::str::from_utf8(&pair[1])
                            .ok()
                            .and_then(|v| v.parse::<u16>().ok())
                            .is_none_or(|v| v == 0 || v > 200)
                        {
                            return Err(AppError::invalid("SCAN COUNT must be between 1 and 200"));
                        }
                    }
                    "TYPE" if name == "SCAN" => {
                        if ![
                            b"string".as_slice(),
                            b"hash",
                            b"list",
                            b"set",
                            b"zset",
                            b"stream",
                        ]
                        .contains(&pair[1].as_slice())
                        {
                            return Err(AppError::invalid("Unsupported SCAN TYPE"));
                        }
                    }
                    _ => return Err(AppError::invalid("Unsupported SCAN option")),
                }
            }
            true
        }
        "SET" => {
            range(2, 3)?;
            if n == 3
                && ![b"KEEPTTL".as_slice(), b"NX", b"XX"]
                    .iter()
                    .any(|v| args[3].eq_ignore_ascii_case(v))
            {
                return Err(AppError::invalid("SET supports KEEPTTL, NX or XX"));
            }
            false
        }
        "HSET" => {
            range(3, 101)?;
            if n % 2 != 1 {
                return Err(AppError::invalid("HSET needs field/value pairs"));
            }
            false
        }
        "LSET" => {
            exact(3)?;
            integer(2)?;
            false
        }
        "ZADD" => {
            range(3, 101)?;
            if n % 2 != 1 {
                return Err(AppError::invalid("ZADD needs score/member pairs"));
            }
            for pair in args[2..].chunks(2) {
                if std::str::from_utf8(&pair[0])
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
                    .is_none_or(|v| !v.is_finite())
                {
                    return Err(AppError::invalid("Use a finite sorted-set score"));
                }
            }
            false
        }
        "HDEL" | "SADD" | "SREM" | "ZREM" | "LPUSH" | "RPUSH" => {
            range(2, 101)?;
            false
        }
        "DEL" | "UNLINK" => {
            range(1, 100)?;
            false
        }
        "EXPIRE" | "PEXPIRE" => {
            exact(2)?;
            if integer(2)? < 0 {
                return Err(AppError::invalid("Expiry must be a nonnegative integer"));
            }
            false
        }
        "PERSIST" => {
            exact(1)?;
            false
        }
        _ => {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "This Redis command is unavailable. Blocking, scripting, transaction, session, administrative and unbounded collection commands are disabled.",
            ));
        }
    };
    Ok(if read {
        StatementKind::Read
    } else {
        StatementKind::Write
    })
}

// Always quote bytes so a selected key cannot inject a second command.
pub fn argument(bytes: &[u8]) -> String {
    let mut value = String::from("\"");
    for b in bytes {
        match b {
            b'"' => value.push_str("\\\""),
            b'\\' => value.push_str("\\\\"),
            32..=126 => value.push(*b as char),
            _ => value.push_str(&format!("\\x{b:02x}")),
        }
    }
    value.push('"');
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_arguments_round_trip_without_command_injection() {
        let bytes = b"a\0\xff\n\" \\";
        let command = parse(&format!("GET {}", argument(bytes))).unwrap();
        assert_eq!(command.len(), 1);
        assert_eq!(command[0].args[1], bytes);
        assert!(parse("GET \"unterminated").is_err());
        assert!(parse("LRANGE k 0 -1").is_err());
        assert!(parse("SCAN 0 COUNT 201").is_err());
        assert!(parse("SCAN 0 COUNT 1 COUNT 2").is_err());
    }
}
