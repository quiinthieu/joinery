use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SmokeOptions {
    pub cleanup_cycles: u32,
    pub idle_ms: u32,
}

impl Default for SmokeOptions {
    fn default() -> Self {
        Self {
            cleanup_cycles: 10,
            idle_ms: 1000,
        }
    }
}

impl SmokeOptions {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > 1024 {
            return Err("Native test options are too large");
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| "Invalid native test options")?;
        if !(10..=1000).contains(&value.cleanup_cycles) || !(1000..=60_000).contains(&value.idle_ms)
        {
            return Err("Native test options exceed supported bounds");
        }
        Ok(value)
    }

    pub fn timeout_seconds(self) -> u64 {
        30 + u64::from(self.cleanup_cycles) + 2 * u64::from(self.idle_ms).div_ceil(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_options_bound_sustained_work_and_watchdog() {
        let options = SmokeOptions::parse(br#"{"cleanupCycles":100,"idleMs":15000}"#).unwrap();
        assert_eq!(options.timeout_seconds(), 160);
        let defaults = SmokeOptions::default();
        assert_eq!(defaults.cleanup_cycles, 10);
        assert_eq!(defaults.idle_ms, 1000);
        for input in [
            r#"{"cleanupCycles":0,"idleMs":1000}"#,
            r#"{"cleanupCycles":1001,"idleMs":1000}"#,
            r#"{"cleanupCycles":10,"idleMs":999}"#,
            r#"{"cleanupCycles":10,"idleMs":60001}"#,
            r#"{"cleanupCycles":10,"idleMs":1000,"unexpected":true}"#,
            r#"{"cleanupCycles":10}"#,
        ] {
            assert!(SmokeOptions::parse(input.as_bytes()).is_err());
        }
        assert!(SmokeOptions::parse(&[b' '; 1025]).is_err());
    }
}
