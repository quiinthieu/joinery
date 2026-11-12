use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryStage {
    Setup,
    Verify,
    EncryptedSetup,
    EncryptedVerify,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SmokeOptions {
    pub cleanup_cycles: u32,
    pub idle_ms: u32,
    #[serde(default)]
    pub recovery_stage: Option<RecoveryStage>,
    #[serde(default)]
    pub fixture_port: Option<u16>,
}

impl Default for SmokeOptions {
    fn default() -> Self {
        Self {
            cleanup_cycles: 10,
            idle_ms: 1000,
            recovery_stage: None,
            fixture_port: None,
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
        if !(10..=1000).contains(&value.cleanup_cycles)
            || !(1000..=60_000).contains(&value.idle_ms)
            || value.fixture_port.is_some_and(|port| port < 1024)
        {
            return Err("Native test options exceed supported bounds");
        }
        Ok(value)
    }

    pub fn timeout_seconds(self) -> u64 {
        match self.recovery_stage {
            Some(RecoveryStage::EncryptedSetup) => return 900,
            Some(RecoveryStage::EncryptedVerify) => return 120,
            Some(RecoveryStage::Setup | RecoveryStage::Verify) => return 90,
            None => {}
        }
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

    #[test]
    fn recovery_options_are_explicit_and_use_only_unprivileged_fixture_ports() {
        assert!(SmokeOptions::default().recovery_stage.is_none());
        for stage in ["setup", "verify"] {
            let options = SmokeOptions::parse(format!(r#"{{"cleanupCycles":10,"idleMs":1000,"recoveryStage":"{stage}","fixturePort":55432}}"#).as_bytes()).unwrap();
            assert_eq!(options.fixture_port, Some(55432));
            assert_eq!(options.timeout_seconds(), 90);
        }
        let defaults =
            SmokeOptions::parse(br#"{"cleanupCycles":10,"idleMs":1000,"recoveryStage":"setup"}"#)
                .unwrap();
        assert_eq!(defaults.recovery_stage, Some(RecoveryStage::Setup));
        assert!(defaults.fixture_port.is_none());
        for extra in [
            r#""recoveryStage":"other""#,
            r#""fixturePort":0"#,
            r#""fixturePort":1023"#,
            r#""fixturePort":65536"#,
        ] {
            assert!(
                SmokeOptions::parse(
                    format!(r#"{{"cleanupCycles":10,"idleMs":1000,{extra}}}"#).as_bytes()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn encrypted_recovery_has_separate_bounded_production_kdf_deadlines() {
        for (stage, seconds) in [("encryptedSetup", 900), ("encryptedVerify", 120)] {
            let options = SmokeOptions::parse(
                format!(r#"{{"cleanupCycles":10,"idleMs":1000,"recoveryStage":"{stage}"}}"#)
                    .as_bytes(),
            )
            .unwrap();
            assert_eq!(options.timeout_seconds(), seconds);
            assert!(options.fixture_port.is_none());
        }
    }
}
