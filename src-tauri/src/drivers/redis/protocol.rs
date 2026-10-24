use crate::domain::*;
use redis::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_BYTES: usize = 2 * 1024 * 1024;
trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

pub struct Wire {
    stream: Box<dyn Stream>,
}
impl Wire {
    pub fn new(stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static) -> Self {
        Self {
            stream: Box::new(stream),
        }
    }
    pub async fn request(&mut self, args: &[Vec<u8>]) -> AppResult<Value> {
        let mut command = redis::cmd(
            std::str::from_utf8(&args[0]).map_err(|_| AppError::invalid("Invalid command name"))?,
        );
        for arg in &args[1..] {
            command.arg(arg);
        }
        self.stream
            .write_all(&command.get_packed_command())
            .await
            .map_err(|_| disconnected())?;
        self.stream.flush().await.map_err(|_| disconnected())?;
        let mut bytes = Vec::with_capacity(8192);
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(end) = frame(&bytes, 0, 0, &mut 0)? {
                if end != bytes.len() {
                    return Err(protocol_error());
                }
                let value = redis::parse_redis_value(&bytes).map_err(|_| protocol_error())?;
                return match value {
                    Value::ServerError(_) => Err(AppError::new(
                        ErrorCode::Database,
                        "Redis rejected the command. Check the key type, command arguments, ACL permissions and standalone endpoint.",
                    )),
                    value => Ok(value),
                };
            }
            if bytes.len() >= MAX_BYTES {
                return Err(limit());
            }
            let capacity = (MAX_BYTES - bytes.len()).min(chunk.len());
            let count = self
                .stream
                .read(&mut chunk[..capacity])
                .await
                .map_err(|_| disconnected())?;
            if count == 0 {
                return Err(disconnected());
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
    }
}

// Validate RESP2 framing and declared lengths before the library allocates any
// response values. No background multiplexed task survives cancellation.
fn frame(bytes: &[u8], at: usize, depth: usize, items: &mut usize) -> AppResult<Option<usize>> {
    if depth > 16 || *items >= 10_000 {
        return Err(limit());
    }
    let Some(&tag) = bytes.get(at) else {
        return Ok(None);
    };
    *items += 1;
    let Some(line_end) = bytes[at + 1..]
        .windows(2)
        .position(|w| w == b"\r\n")
        .map(|p| at + 1 + p)
    else {
        if bytes.len() - at > MAX_BYTES {
            return Err(limit());
        }
        return Ok(None);
    };
    let end = line_end + 2;
    match tag {
        b'+' | b'-' | b':' => Ok(Some(end)),
        b'$' | b'*' => {
            let n = std::str::from_utf8(&bytes[at + 1..line_end])
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(protocol_error)?;
            if n == -1 {
                return Ok(Some(end));
            }
            if n < 0 {
                return Err(protocol_error());
            }
            let n = usize::try_from(n).map_err(|_| limit())?;
            if tag == b'$' {
                if n > MAX_BYTES || end.saturating_add(n).saturating_add(2) > MAX_BYTES {
                    return Err(limit());
                }
                let finish = end + n + 2;
                if bytes.len() < finish {
                    return Ok(None);
                }
                if &bytes[finish - 2..finish] != b"\r\n" {
                    return Err(protocol_error());
                }
                Ok(Some(finish))
            } else {
                if n > 10_000 {
                    return Err(limit());
                }
                let mut pos = end;
                for _ in 0..n {
                    match frame(bytes, pos, depth + 1, items)? {
                        Some(next) => pos = next,
                        None => return Ok(None),
                    }
                }
                Ok(Some(pos))
            }
        }
        _ => Err(protocol_error()),
    }
}
pub fn disconnected() -> AppError {
    AppError::new(
        ErrorCode::ConnectionClosed,
        "The Redis connection closed. Reconnect and verify any write outcome before retrying.",
    )
}
fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "The Redis response exceeds the 2 MiB / 10,000 element / 16-level viewer budget. Use a smaller range or SCAN count.",
    )
}
fn protocol_error() -> AppError {
    AppError::new(
        ErrorCode::ConnectionClosed,
        "Redis returned an invalid or unsupported RESP2 response",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_rejects_declared_oversize_and_nested_allocations_before_decoding() {
        assert!(frame(b"$2147483647\r\n", 0, 0, &mut 0).is_err());
        assert!(frame(b"*1000000000\r\n", 0, 0, &mut 0).is_err());
        let nested = "*1\r\n".repeat(18) + "+OK\r\n";
        assert!(frame(nested.as_bytes(), 0, 0, &mut 0).is_err());
        let response = b"*3\r\n$0\r\n\r\n$-1\r\n:9223372036854775807\r\n";
        for end in 0..response.len() {
            assert_eq!(frame(&response[..end], 0, 0, &mut 0).unwrap(), None);
        }
        assert_eq!(frame(response, 0, 0, &mut 0).unwrap(), Some(response.len()));
    }
}
