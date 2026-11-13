//! Closed mutation-batch protocol evidence. This validates helper observations;
//! it does not infer transaction disposition from a successful statement ACK.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// SQL-free statement identity shared with the Unix helper transport. Keeping
/// the value portable lets historical captures be inspected without a helper.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatementIdentity {
    pub sdk_request_id: String,
    pub query_id: Option<String>,
}

pub const MAX_UNITS: usize = 500;
const MAX_CONTROL_STEPS: usize = 10;
pub const MAX_STEPS: usize = MAX_UNITS + MAX_CONTROL_STEPS;
pub const MAX_MANUAL_BATCHES: u64 = 65_536;
pub const MAX_CAPTURE_BYTES: usize = 32 * 1024;

/// Durable binary format v1. Store using a byte-safe encoding (base64), never
/// as a JSON array of byte numbers. Decode revalidates the entire event prefix.
pub struct BatchCapture;
impl BatchCapture {
    pub const VERSION: u8 = 1;

    /// Conservative byte bound for the canonical binary capture of every
    /// accepted prefix and optional terminal for this closed request. This
    /// validates the public descriptor's shape, not SQL or execution authority.
    /// No surrounding durable envelope or byte-safe encoding is included.
    pub fn maximum_encoded_len(request: &BatchRequest) -> Result<usize, &'static str> {
        request.validate_capture_shape()?;
        let units = request.units();
        let descriptor = 4 + match request {
            BatchRequest::Batch { owner, .. } => {
                // Request tag, mode, rollback permission, optional owner,
                // unit-count varint and one kind tag for each unit.
                3 + 1 + owner.map_or(0, |_| 16) + number_len(units.len() as u128) + units.len()
            }
            BatchRequest::Finish { .. } => 2 + 1 + 16,
        };
        // Each step has role/phase tags, its unit-index varint, a statement
        // flag, the exact SDK UUID and optional query UUID, and a count tag.
        // Missing identity/ACK/count/drain evidence only shortens this form.
        const STEP_FIXED: usize = 2 + 1 + 16 + 1 + 16 + 1;
        let controls = MAX_CONTROL_STEPS * (STEP_FIXED + number_len(0));
        let unit_bytes: usize = units
            .iter()
            .enumerate()
            .map(|(index, kind)| {
                STEP_FIXED
                    + number_len(index as u128 + 1)
                    + match kind {
                        UnitKind::Update => 3, // exact 1, presence flag, exact 0
                        _ => 2,                // exact 1 and absent multi-joined count
                    }
            })
            .sum();
        // apply_event requires all preceding units to have exact successful
        // counts before dispatching another unit. Thus at most the last unit
        // can retain arbitrary 38-digit counts, even for Unknown terminals.
        // Count-unavailable markers and count-less SQL rejections are smaller.
        let wide_count_extra = if units.is_empty() {
            0
        } else {
            let extra = number_len(10u128.pow(38) - 1) - number_len(1);
            extra
                * if units.contains(&UnitKind::Update) {
                    2
                } else {
                    1
                }
        };
        let summary_owner = match request {
            BatchRequest::Batch {
                mode: Mode::Automatic,
                ..
            } => 0,
            _ => 16,
        };
        // Present summary/outcome, optional owner, failure tag, maximum prior
        // count, present transaction, and the full signed-i64 zigzag varint.
        let summary = 1
            + 1
            + 1
            + summary_owner
            + 1
            + number_len(u128::from(MAX_MANUAL_BATCHES))
            + 1
            + number_len(u128::from(u64::MAX));
        Ok(descriptor
            + number_len((units.len() + MAX_CONTROL_STEPS) as u128)
            + controls
            + unit_bytes
            + wide_count_extra
            + summary)
    }

    /// Padded STANDARD base64 byte bound, excluding quotes and the complete
    /// durable envelope. Callers must account for that envelope before SQL.
    pub fn maximum_base64_len(request: &BatchRequest) -> Result<usize, &'static str> {
        Ok(Self::maximum_encoded_len(request)?.div_ceil(3) * 4)
    }

    pub fn encode(evidence: &BatchEvidence) -> Result<Vec<u8>, &'static str> {
        evidence.validate_prefix()?;
        let bytes = evidence.compact_capture()?;
        if Self::decode(&bytes)? != *evidence {
            return Err("batch capture roundtrip mismatch");
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<BatchEvidence, &'static str> {
        if bytes.len() > MAX_CAPTURE_BYTES
            || bytes.get(..3) != Some(b"JMB")
            || bytes.get(3) != Some(&Self::VERSION)
        {
            return Err("invalid batch capture");
        }
        let mut input = CaptureReader { bytes: &bytes[4..] };
        let request = match input.byte()? {
            0 => {
                let mode = input.tag(&[Mode::Automatic, Mode::Manual])?;
                let rollback_prior = input.flag()?;
                let owner = input.uuid()?;
                if mode == Mode::Automatic && (owner.is_some() || rollback_prior)
                    || mode == Mode::Manual && rollback_prior != owner.is_some()
                {
                    return Err("invalid batch owner");
                }
                let count = input.bounded(MAX_UNITS)?;
                if count == 0 {
                    return Err("empty batch capture");
                }
                let mut units = Vec::with_capacity(count);
                for _ in 0..count {
                    units.push(input.tag(&[
                        UnitKind::Insert,
                        UnitKind::Update,
                        UnitKind::Delete,
                    ])?);
                }
                BatchRequest::Batch {
                    mode,
                    owner,
                    rollback_prior,
                    units,
                }
            }
            1 => {
                let action = input.tag(&[FinishAction::Commit, FinishAction::Rollback])?;
                let owner = input.uuid()?.ok_or("missing finish owner")?;
                BatchRequest::Finish { owner, action }
            }
            _ => return Err("invalid capture request"),
        };
        let count = input.bounded(MAX_STEPS)?;
        let mut steps = Vec::with_capacity(count);
        for index in 0..count {
            let role = input.tag(&[
                Role::Context,
                Role::Settings,
                Role::Begin,
                Role::Transaction,
                Role::Unit,
                Role::Verify,
                Role::Commit,
                Role::Rollback,
                Role::Idle,
                Role::Disposition,
            ])?;
            let phase = input.tag(&[
                StepPhase::Dispatching,
                StepPhase::Acknowledged,
                StepPhase::Drained,
            ])?;
            let unit = input.bounded(MAX_UNITS)?;
            let unit = (unit != 0).then(|| (unit - 1) as u16);
            let statement = if input.flag()? {
                Some(StatementIdentity {
                    sdk_request_id: input.required_uuid()?.to_string(),
                    query_id: input.uuid()?.map(|id| id.to_string()),
                })
            } else {
                None
            };
            let count_tag = input.byte()?;
            let (count, count_unavailable) = match count_tag {
                0 => (None, false),
                1 => (
                    Some(ExactCount {
                        affected_rows: input.count()?,
                        multi_joined_rows: if input.flag()? {
                            Some(input.count()?)
                        } else {
                            None
                        },
                    }),
                    false,
                ),
                2 => (None, true),
                _ => return Err("invalid capture count tag"),
            };
            steps.push(BatchStep {
                step: index as u16 + 1,
                role,
                unit,
                phase,
                statement,
                count,
                count_unavailable,
            });
        }
        let summary = if input.flag()? {
            let outcome = input.tag(&[
                Outcome::Committed,
                Outcome::Pending,
                Outcome::RolledBack,
                Outcome::Rejected,
                Outcome::Unknown,
            ])?;
            let owner = input.uuid()?;
            let failure = match input.byte()? {
                0 => None,
                n => Some(
                    *[
                        Failure::Admission,
                        Failure::Cardinality,
                        Failure::Database,
                        Failure::Cancelled,
                        Failure::Transport,
                        Failure::Context,
                        Failure::Observer,
                        Failure::SummaryUnavailable,
                    ]
                    .get(usize::from(n - 1))
                    .ok_or("invalid failure")?,
                ),
            };
            let prior_batches = input.number()?;
            if prior_batches > u128::from(MAX_MANUAL_BATCHES) {
                return Err("invalid prior batch count");
            }
            let transaction_id = if input.flag()? {
                let zigzag = u64::try_from(input.number()?).map_err(|_| "invalid transaction")?;
                Some(((zigzag >> 1) as i64 ^ -((zigzag & 1) as i64)).to_string())
            } else {
                None
            };
            Some(BatchSummary {
                outcome,
                owner,
                transaction_id,
                failure,
                prior_batches: prior_batches as u64,
            })
        } else {
            None
        };
        if !input.bytes.is_empty() {
            return Err("trailing batch capture bytes");
        }
        let evidence = BatchEvidence {
            request,
            steps,
            summary,
        };
        evidence.validate_prefix()?;
        // Reject alternate/noncanonical encodings as well as malformed values.
        if evidence.compact_capture()? != bytes {
            return Err("noncanonical batch capture");
        }
        Ok(evidence)
    }
}

struct CaptureReader<'a> {
    bytes: &'a [u8],
}
impl CaptureReader<'_> {
    fn byte(&mut self) -> Result<u8, &'static str> {
        let (&byte, rest) = self.bytes.split_first().ok_or("truncated batch capture")?;
        self.bytes = rest;
        Ok(byte)
    }
    fn flag(&mut self) -> Result<bool, &'static str> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("invalid capture flag"),
        }
    }
    fn tag<T: Copy>(&mut self, values: &[T]) -> Result<T, &'static str> {
        values
            .get(usize::from(self.byte()?))
            .copied()
            .ok_or("invalid capture tag")
    }
    fn required_uuid(&mut self) -> Result<Uuid, &'static str> {
        let value: [u8; 16] = self
            .bytes
            .get(..16)
            .ok_or("truncated UUID")?
            .try_into()
            .map_err(|_| "invalid UUID")?;
        self.bytes = &self.bytes[16..];
        Ok(Uuid::from_bytes(value))
    }
    fn uuid(&mut self) -> Result<Option<Uuid>, &'static str> {
        if self.flag()? {
            self.required_uuid().map(Some)
        } else {
            Ok(None)
        }
    }
    fn number(&mut self) -> Result<u128, &'static str> {
        let mut number = 0u128;
        for shift in (0..=126).step_by(7) {
            let byte = self.byte()?;
            if shift == 126 && byte > 3 {
                return Err("capture number overflow");
            }
            number |= u128::from(byte & 0x7f) << shift;
            if byte < 128 {
                return Ok(number);
            }
        }
        Err("capture number overflow")
    }
    fn bounded(&mut self, maximum: usize) -> Result<usize, &'static str> {
        let number = self.number()?;
        if number > maximum as u128 {
            return Err("capture count exceeds bound");
        }
        Ok(number as usize)
    }
    fn count(&mut self) -> Result<String, &'static str> {
        let number = self.number()?.to_string();
        if number.len() > 38 {
            return Err("capture decimal exceeds bound");
        }
        Ok(number)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Automatic,
    Manual,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum UnitKind {
    Insert,
    Update,
    Delete,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum FinishAction {
    Commit,
    Rollback,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchRequest {
    Batch {
        mode: Mode,
        owner: Option<Uuid>,
        rollback_prior: bool,
        units: Vec<UnitKind>,
    },
    Finish {
        owner: Uuid,
        action: FinishAction,
    },
}

fn object_keys(value: &Value, keys: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
    })
}
fn uuid(value: &Value) -> Option<Uuid> {
    let text = value.as_str()?;
    let id = Uuid::parse_str(text).ok()?;
    (text == id.hyphenated().to_string()).then_some(id)
}
fn optional_uuid(value: &Value) -> Result<Option<Uuid>, &'static str> {
    if value.is_null() {
        Ok(None)
    } else {
        uuid(value)
            .filter(valid_owner)
            .map(Some)
            .ok_or("invalid mutation batch")
    }
}
fn valid_owner(id: &Uuid) -> bool {
    matches!(id.get_version_num(), 1..=5) && id.get_variant() == uuid::Variant::RFC4122
}
fn expected(value: &Value) -> bool {
    object_keys(
        value,
        &["user", "session", "role", "database", "schema", "warehouse"],
    ) && value["user"].as_str().is_some_and(|s| {
        !s.is_empty() && s.len() <= 1020 && s.chars().count() <= 255 && !s.contains('\0')
    }) && value["session"]
        .as_str()
        .is_some_and(|s| !s.is_empty() && s.len() <= 1024 && !s.contains('\0'))
        && ["role", "database", "schema", "warehouse"]
            .iter()
            .all(|key| {
                value[*key].is_null()
                    || value[*key].as_str().is_some_and(|s| {
                        !s.is_empty()
                            && s.len() <= 1020
                            && s.chars().count() <= 255
                            && !s.contains('\0')
                    })
            })
}

impl BatchRequest {
    fn validate_capture_shape(&self) -> Result<(), &'static str> {
        match self {
            Self::Batch {
                mode,
                owner,
                rollback_prior,
                units,
            } => {
                if units.is_empty()
                    || units.len() > MAX_UNITS
                    || *mode == Mode::Automatic && (owner.is_some() || *rollback_prior)
                    || *mode == Mode::Manual && *rollback_prior != owner.is_some()
                    || owner.as_ref().is_some_and(|owner| !valid_owner(owner))
                {
                    return Err("invalid capture descriptor");
                }
            }
            Self::Finish { owner, .. } if !valid_owner(owner) => {
                return Err("invalid finish owner");
            }
            Self::Finish { .. } => {}
        }
        Ok(())
    }

    /// Called before serialization or queue admission. SQL/binds are validated
    /// here but deliberately never copied into retained protocol evidence.
    pub fn parse(op: &str, payload: &Value) -> Result<Option<Self>, &'static str> {
        match op {
            "mutationBatch" => {
                if !object_keys(
                    payload,
                    &["mode", "owner", "rollbackPrior", "expected", "units"],
                ) || !expected(&payload["expected"])
                {
                    return Err("invalid mutation batch");
                }
                let mode: Mode = serde_json::from_value(payload["mode"].clone())
                    .map_err(|_| "invalid mutation batch")?;
                let owner = optional_uuid(&payload["owner"])?;
                let rollback_prior = payload["rollbackPrior"]
                    .as_bool()
                    .ok_or("invalid mutation batch")?;
                if (mode == Mode::Automatic && (owner.is_some() || rollback_prior))
                    || (mode == Mode::Manual && rollback_prior != owner.is_some())
                {
                    return Err("invalid mutation batch");
                }
                let source = payload["units"]
                    .as_array()
                    .ok_or("invalid mutation batch")?;
                if source.is_empty() || source.len() > MAX_UNITS {
                    return Err("invalid mutation batch");
                }
                let mut units = Vec::with_capacity(source.len());
                for unit in source {
                    if !object_keys(unit, &["kind", "sql", "binds"])
                        || !unit["sql"].as_str().is_some_and(|s| {
                            !s.trim().is_empty() && s.len() <= 262_144 && !s.contains('\0')
                        })
                        || !unit["binds"].as_array().is_some_and(|binds| {
                            binds.len() <= 1000
                                && binds.iter().all(|bind| {
                                    bind.is_null()
                                        || bind.is_boolean()
                                        || bind.as_str().is_some_and(|s| s.len() <= 262_144)
                                })
                        })
                    {
                        return Err("invalid mutation batch");
                    }
                    units.push(
                        serde_json::from_value(unit["kind"].clone())
                            .map_err(|_| "invalid mutation batch")?,
                    );
                }
                Ok(Some(Self::Batch {
                    mode,
                    owner,
                    rollback_prior,
                    units,
                }))
            }
            "mutationFinish" => {
                if !object_keys(payload, &["owner", "action", "expected"])
                    || !expected(&payload["expected"])
                {
                    return Err("invalid mutation batch");
                }
                Ok(Some(Self::Finish {
                    owner: uuid(&payload["owner"])
                        .filter(valid_owner)
                        .ok_or("invalid mutation batch")?,
                    action: serde_json::from_value(payload["action"].clone())
                        .map_err(|_| "invalid mutation batch")?,
                }))
            }
            _ => Ok(None),
        }
    }
    pub fn owner(&self) -> Option<Uuid> {
        match self {
            Self::Batch { owner, .. } => *owner,
            Self::Finish { owner, .. } => Some(*owner),
        }
    }
    pub fn units(&self) -> &[UnitKind] {
        match self {
            Self::Batch { units, .. } => units,
            Self::Finish { .. } => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Context,
    Settings,
    Begin,
    Transaction,
    Unit,
    Verify,
    Commit,
    Rollback,
    Idle,
    Disposition,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "camelCase")]
pub enum StepPhase {
    Dispatching,
    Acknowledged,
    Drained,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactCount {
    pub affected_rows: String,
    pub multi_joined_rows: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchStep {
    pub step: u16,
    pub role: Role,
    pub unit: Option<u16>,
    pub phase: StepPhase,
    pub statement: Option<StatementIdentity>,
    pub count: Option<ExactCount>,
    pub count_unavailable: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Committed,
    Pending,
    RolledBack,
    Rejected,
    Unknown,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Failure {
    Admission,
    Cardinality,
    Database,
    Cancelled,
    Transport,
    Context,
    Observer,
    SummaryUnavailable,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchSummary {
    pub outcome: Outcome,
    pub owner: Option<Uuid>,
    pub transaction_id: Option<String>,
    pub failure: Option<Failure>,
    pub prior_batches: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchEvidence {
    pub request: BatchRequest,
    pub steps: Vec<BatchStep>,
    pub summary: Option<BatchSummary>,
}
fn decimal(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    (!value.is_empty()
        && value.len() <= 38
        && value.bytes().all(|c| c.is_ascii_digit())
        && (value == "0" || !value.starts_with('0')))
    .then(|| value.to_owned())
}
fn event_keys(event: &Value, extra: &[&str], optional: &[&str]) -> bool {
    event.as_object().is_some_and(|object| {
        ["v", "generation", "id", "type"]
            .iter()
            .chain(extra)
            .all(|key| object.contains_key(*key))
            && object.keys().all(|key| {
                ["v", "generation", "id", "type"].contains(&key.as_str())
                    || extra.contains(&key.as_str())
                    || optional.contains(&key.as_str())
            })
    })
}
impl BatchEvidence {
    pub fn new(request: BatchRequest) -> Self {
        Self {
            request,
            steps: vec![],
            summary: None,
        }
    }
    pub fn has_acknowledgement(&self) -> bool {
        self.steps.iter().any(|step| {
            matches!(
                step.role,
                Role::Unit | Role::Begin | Role::Commit | Role::Rollback
            ) && step.phase >= StepPhase::Acknowledged
        })
    }
    pub fn has_dispatch(&self) -> bool {
        !self.steps.is_empty()
    }
    fn validate_prefix(&self) -> Result<(), &'static str> {
        self.request.validate_capture_shape()?;
        let mut replay = Self::new(self.request.clone());
        let envelope = |extra: Value| {
            let mut frame = serde_json::json!({"v":1,"generation":"capture","id":"capture"});
            frame
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            frame
        };
        for step in &self.steps {
            let phase = |phase| {
                envelope(serde_json::json!({"type":"batchStep","step":step.step,
                "role":step.role,"unit":step.unit,"phase":phase}))
            };
            if !replay.apply_event(&phase(StepPhase::Dispatching)) {
                return Err("invalid capture step");
            }
            if let Some(statement) = &step.statement {
                let mut event = envelope(
                    serde_json::json!({"type":"batchStatement","step":step.step,"sdkRequestId":statement.sdk_request_id}),
                );
                if let Some(query_id) = &statement.query_id {
                    event["queryId"] = Value::String(query_id.clone());
                }
                if !replay.apply_event(&event) {
                    return Err("invalid capture identity");
                }
            }
            if step.phase >= StepPhase::Acknowledged
                && !replay.apply_event(&phase(StepPhase::Acknowledged))
            {
                return Err("invalid capture ACK");
            }
            if let Some(count) = &step.count
                && !replay.apply_event(&envelope(
                    serde_json::json!({"type":"batchCount","step":step.step,"unit":step.unit,
                    "affectedRows":count.affected_rows,"multiJoinedRows":count.multi_joined_rows}),
                ))
            {
                return Err("invalid capture count");
            }
            if step.count_unavailable
                && !replay.apply_event(&envelope(serde_json::json!({
                "type":"batchCountUnavailable","step":step.step,"unit":step.unit})))
            {
                return Err("invalid capture count absence");
            }
            if step.phase == StepPhase::Drained && !replay.apply_event(&phase(StepPhase::Drained)) {
                return Err("invalid capture drain");
            }
        }
        if let Some(summary) = &self.summary
            && !replay.accept_terminal(
                &serde_json::to_value(summary).map_err(|_| "invalid capture summary")?,
            )
        {
            return Err("invalid capture summary");
        }
        if replay != *self {
            return Err("invalid capture prefix");
        }
        Ok(())
    }
    fn next_role(&self, role: Role) -> bool {
        let roles: Vec<_> = self.steps.iter().map(|step| step.role).collect();
        if roles.contains(&Role::Rollback) {
            return match roles.last() {
                Some(Role::Rollback) => role == Role::Idle,
                Some(Role::Idle) => role == Role::Disposition,
                _ => false,
            };
        }
        // A known rejection can roll back the whole owned transaction. An
        // interrupted ACK/drain is not eligible: dispatch requires prior drain.
        if role == Role::Rollback {
            return match self.request {
                BatchRequest::Finish {
                    action: FinishAction::Rollback,
                    ..
                } => roles.is_empty(),
                _ => {
                    !roles.contains(&Role::Commit)
                        && (self.request.owner().is_some() || roles.contains(&Role::Begin))
                }
            };
        }
        let prefix: &[Role] = match &self.request {
            BatchRequest::Batch { owner: None, .. } => &[
                Role::Context,
                Role::Settings,
                Role::Begin,
                Role::Transaction,
            ],
            BatchRequest::Batch { owner: Some(_), .. } => &[Role::Context, Role::Settings],
            BatchRequest::Finish {
                action: FinishAction::Commit,
                ..
            } => &[
                Role::Context,
                Role::Settings,
                Role::Commit,
                Role::Idle,
                Role::Disposition,
            ],
            BatchRequest::Finish {
                action: FinishAction::Rollback,
                ..
            } => return false,
        };
        if roles.len() < prefix.len() {
            return role == prefix[roles.len()];
        }
        if matches!(self.request, BatchRequest::Finish { .. }) {
            return false;
        }
        let dispatched_units = self.steps.iter().filter(|s| s.role == Role::Unit).count();
        if dispatched_units < self.request.units().len() {
            return role == Role::Unit;
        }
        let tail = roles.len() - prefix.len() - dispatched_units;
        let suffix: &[Role] = match self.request {
            BatchRequest::Batch {
                mode: Mode::Automatic,
                ..
            } => &[Role::Verify, Role::Commit, Role::Idle, Role::Disposition],
            _ => &[Role::Verify],
        };
        if suffix.get(tail) != Some(&role) {
            return false;
        }
        self.steps.iter().filter(|s| s.role == Role::Unit).all(|s| {
            s.count.as_ref().is_some_and(|c| {
                c.affected_rows == "1" && c.multi_joined_rows.as_deref().is_none_or(|n| n == "0")
            })
        })
    }
    /// Atomic on failure: callers may retire the process while preserving the
    /// last valid evidence, rather than accepting a partially mutated frame.
    pub fn apply_event(&mut self, event: &Value) -> bool {
        // Each branch validates before its single final mutation.
        self.apply(event)
    }
    fn apply(&mut self, event: &Value) -> bool {
        if self.summary.is_some() {
            return false;
        }
        let Some(number) = event["step"]
            .as_u64()
            .filter(|n| (1..=MAX_STEPS as u64).contains(n))
        else {
            return false;
        };
        let number = number as u16;
        match event["type"].as_str() {
            Some("batchStep") => {
                if !event_keys(event, &["step", "role", "unit", "phase"], &[]) {
                    return false;
                }
                let Ok(role) = serde_json::from_value::<Role>(event["role"].clone()) else {
                    return false;
                };
                let Ok(phase) = serde_json::from_value::<StepPhase>(event["phase"].clone()) else {
                    return false;
                };
                let unit = if event["unit"].is_null() {
                    None
                } else {
                    let Some(unit) = event["unit"]
                        .as_u64()
                        .filter(|u| *u < self.request.units().len() as u64)
                    else {
                        return false;
                    };
                    Some(unit as u16)
                };
                if (role == Role::Unit) != unit.is_some() {
                    return false;
                }
                if phase == StepPhase::Dispatching {
                    if self.steps.len() >= MAX_STEPS
                        || usize::from(number) != self.steps.len() + 1
                        || self.steps.last().is_some_and(|step| {
                            step.phase != StepPhase::Drained
                                && !(role == Role::Rollback
                                    && step.role == Role::Unit
                                    && step.phase == StepPhase::Dispatching
                                    && step.statement.is_some())
                        })
                        || !self.next_role(role)
                        || (role != Role::Unit
                            && self.steps.iter().filter(|s| s.role != Role::Unit).count()
                                >= MAX_CONTROL_STEPS)
                    {
                        return false;
                    }
                    if let Some(unit) = unit
                        && (usize::from(unit)
                            != self.steps.iter().filter(|s| s.role == Role::Unit).count()
                            || self.steps.iter().filter(|s| s.role == Role::Unit).any(|s| {
                                !s.count.as_ref().is_some_and(|c| {
                                    c.affected_rows == "1"
                                        && c.multi_joined_rows.as_deref().is_none_or(|n| n == "0")
                                })
                            }))
                    {
                        return false;
                    }
                    self.steps.push(BatchStep {
                        step: number,
                        role,
                        unit,
                        phase,
                        statement: None,
                        count: None,
                        count_unavailable: false,
                    });
                    return true;
                }
                let Some(step) = self
                    .steps
                    .last_mut()
                    .filter(|s| s.step == number && s.role == role && s.unit == unit)
                else {
                    return false;
                };
                if !matches!(
                    (step.phase, phase),
                    (StepPhase::Dispatching, StepPhase::Acknowledged)
                        | (StepPhase::Acknowledged, StepPhase::Drained)
                ) {
                    return false;
                }
                if phase == StepPhase::Drained && step.statement.is_none() {
                    return false;
                }
                if phase == StepPhase::Drained
                    && role == Role::Unit
                    && step.count.is_none()
                    && !step.count_unavailable
                {
                    return false;
                }
                step.phase = phase;
                true
            }
            Some("batchStatement") => {
                if !event_keys(event, &["step", "sdkRequestId"], &["queryId"]) {
                    return false;
                }
                let Some(request_id) = uuid(&event["sdkRequestId"]) else {
                    return false;
                };
                let query_id = match event.get("queryId") {
                    None => None,
                    Some(value) => match uuid(value) {
                        Some(id) => Some(id.to_string()),
                        None => return false,
                    },
                };
                let index = usize::from(number) - 1;
                if self.steps.get(index).is_none() {
                    return false;
                }
                let request_id = request_id.to_string();
                if self.steps.iter().enumerate().any(|(i, s)| {
                    i != index
                        && s.statement.as_ref().is_some_and(|s| {
                            s.sdk_request_id == request_id
                                || query_id
                                    .as_ref()
                                    .is_some_and(|q| s.query_id.as_ref() == Some(q))
                        })
                }) {
                    return false;
                }
                let current = index + 1 == self.steps.len();
                let step = &mut self.steps[index];
                match &mut step.statement {
                    None if step.phase <= StepPhase::Acknowledged
                        && step.count.is_none()
                        && !step.count_unavailable
                        && current =>
                    {
                        step.statement = Some(StatementIdentity {
                            sdk_request_id: request_id,
                            query_id,
                        });
                        true
                    }
                    Some(existing)
                        if current
                            && step.phase == StepPhase::Acknowledged
                            && step.count.is_none()
                            && !step.count_unavailable
                            && existing.sdk_request_id == request_id
                            && existing.query_id.is_none()
                            && query_id.is_some() =>
                    {
                        existing.query_id = query_id;
                        true
                    }
                    _ => false,
                }
            }
            Some("batchCount") => {
                if !event_keys(
                    event,
                    &["step", "unit", "affectedRows", "multiJoinedRows"],
                    &[],
                ) {
                    return false;
                }
                let Some(unit) = event["unit"]
                    .as_u64()
                    .filter(|u| *u < self.request.units().len() as u64)
                else {
                    return false;
                };
                let Some(affected_rows) = decimal(&event["affectedRows"]) else {
                    return false;
                };
                let multi_joined_rows = if event["multiJoinedRows"].is_null() {
                    None
                } else {
                    match decimal(&event["multiJoinedRows"]) {
                        Some(n) => Some(n),
                        None => return false,
                    }
                };
                if self.request.units()[unit as usize] != UnitKind::Update
                    && multi_joined_rows.is_some()
                {
                    return false;
                }
                let Some(step) = self.steps.last_mut().filter(|s| {
                    s.step == number
                        && s.unit == Some(unit as u16)
                        && s.phase == StepPhase::Acknowledged
                        && s.statement.is_some()
                        && s.count.is_none()
                        && !s.count_unavailable
                }) else {
                    return false;
                };
                step.count = Some(ExactCount {
                    affected_rows,
                    multi_joined_rows,
                });
                true
            }
            Some("batchCountUnavailable") => {
                if !event_keys(event, &["step", "unit"], &[]) {
                    return false;
                }
                let Some(unit) = event["unit"]
                    .as_u64()
                    .filter(|u| *u < self.request.units().len() as u64)
                else {
                    return false;
                };
                let Some(step) = self.steps.last_mut().filter(|s| {
                    s.step == number
                        && s.unit == Some(unit as u16)
                        && s.phase == StepPhase::Acknowledged
                        && s.statement.is_some()
                        && s.count.is_none()
                        && !s.count_unavailable
                }) else {
                    return false;
                };
                step.count_unavailable = true;
                true
            }
            _ => false,
        }
    }
    pub fn accept_terminal(&mut self, result: &Value) -> bool {
        if self.summary.is_some()
            || !object_keys(
                result,
                &[
                    "outcome",
                    "owner",
                    "transactionId",
                    "failure",
                    "priorBatches",
                ],
            )
        {
            return false;
        }
        let Ok(summary) = serde_json::from_value::<BatchSummary>(result.clone()) else {
            return false;
        };
        if summary.prior_batches > MAX_MANUAL_BATCHES
            || optional_uuid(&result["owner"]).is_err()
            || summary
                .transaction_id
                .as_ref()
                .is_some_and(|id| id.parse::<i64>().ok().is_none_or(|n| n.to_string() != *id))
            || self
                .request
                .owner()
                .is_some_and(|owner| summary.owner != Some(owner))
            || matches!(
                self.request,
                BatchRequest::Batch {
                    mode: Mode::Automatic,
                    ..
                }
            ) && summary.owner.is_some()
            || matches!(
                self.request,
                BatchRequest::Batch {
                    mode: Mode::Manual,
                    ..
                }
            ) && summary.owner.is_none()
        {
            return false;
        }
        let drained = |role| {
            self.steps
                .iter()
                .any(|s| s.role == role && s.phase == StepPhase::Drained)
        };
        let ack = |role| {
            self.steps
                .iter()
                .any(|s| s.role == role && s.phase >= StepPhase::Acknowledged)
        };
        let units_complete = self.steps.iter().filter(|s| s.role == Role::Unit).count()
            == self.request.units().len()
            && self.steps.iter().filter(|s| s.role == Role::Unit).all(|s| {
                s.phase == StepPhase::Drained
                    && s.count.as_ref().is_some_and(|c| {
                        c.affected_rows == "1"
                            && c.multi_joined_rows.as_deref().is_none_or(|n| n == "0")
                    })
            });
        let valid = match summary.outcome {
            Outcome::Committed => {
                summary.failure.is_none()
                    && summary.transaction_id.is_some()
                    && ack(Role::Commit)
                    && drained(Role::Idle)
                    && drained(Role::Disposition)
                    && match self.request {
                        BatchRequest::Batch {
                            mode: Mode::Automatic,
                            ..
                        } => units_complete,
                        BatchRequest::Finish {
                            action: FinishAction::Commit,
                            ..
                        } => true,
                        _ => false,
                    }
            }
            Outcome::Pending => {
                summary.failure.is_none()
                    && summary.owner.is_some()
                    && summary.transaction_id.is_some()
                    && units_complete
                    && drained(Role::Verify)
                    && matches!(
                        self.request,
                        BatchRequest::Batch {
                            mode: Mode::Manual,
                            ..
                        }
                    )
            }
            Outcome::RolledBack => {
                summary.transaction_id.is_some()
                    && ack(Role::Rollback)
                    && drained(Role::Idle)
                    && drained(Role::Disposition)
                    && (self.steps.iter().all(|s| s.phase == StepPhase::Drained)
                        || summary.failure == Some(Failure::Database))
                    && (!self.steps.iter().any(|s| s.count_unavailable)
                        || summary.failure == Some(Failure::SummaryUnavailable))
            }
            Outcome::Rejected => {
                summary.failure.is_some()
                    && !self.steps.iter().any(|s| {
                        matches!(
                            s.role,
                            Role::Unit | Role::Begin | Role::Commit | Role::Rollback
                        )
                    })
            }
            Outcome::Unknown => summary.failure.is_some(),
        };
        if !valid {
            return false;
        }
        self.summary = Some(summary);
        true
    }
    /// Versioned closed binary evidence. UUIDs use their exact 16 bytes and
    /// counts use unsigned base-128 varints; no SQL, bind, or arbitrary error
    /// text can enter this adapter. Refuse oversize evidence without truncation.
    fn compact_capture(&self) -> Result<Vec<u8>, &'static str> {
        let mut bytes = b"JMB\x01".to_vec();
        match &self.request {
            BatchRequest::Batch {
                mode,
                owner,
                rollback_prior,
                units,
            } => {
                bytes.extend([0, *mode as u8, u8::from(*rollback_prior)]);
                put_uuid(&mut bytes, *owner);
                put_number(&mut bytes, units.len() as u128);
                bytes.extend(units.iter().map(|kind| *kind as u8));
            }
            BatchRequest::Finish { owner, action } => {
                bytes.extend([1, *action as u8]);
                put_uuid(&mut bytes, Some(*owner));
            }
        }
        put_number(&mut bytes, self.steps.len() as u128);
        for step in &self.steps {
            bytes.extend([step.role as u8, step.phase as u8]);
            put_number(&mut bytes, step.unit.map_or(0, |u| u128::from(u) + 1));
            bytes.push(u8::from(step.statement.is_some()));
            if let Some(statement) = &step.statement {
                bytes.extend(
                    Uuid::parse_str(&statement.sdk_request_id)
                        .map_err(|_| "invalid identity")?
                        .as_bytes(),
                );
                put_uuid(
                    &mut bytes,
                    statement
                        .query_id
                        .as_ref()
                        .map(|q| Uuid::parse_str(q))
                        .transpose()
                        .map_err(|_| "invalid identity")?,
                );
            }
            bytes.push(if step.count_unavailable {
                2
            } else {
                u8::from(step.count.is_some())
            });
            if let Some(count) = &step.count {
                put_number(
                    &mut bytes,
                    count.affected_rows.parse().map_err(|_| "invalid count")?,
                );
                bytes.push(u8::from(count.multi_joined_rows.is_some()));
                if let Some(count) = &count.multi_joined_rows {
                    put_number(&mut bytes, count.parse().map_err(|_| "invalid count")?);
                }
            }
        }
        bytes.push(u8::from(self.summary.is_some()));
        if let Some(summary) = &self.summary {
            bytes.push(summary.outcome as u8);
            put_uuid(&mut bytes, summary.owner);
            bytes.push(summary.failure.map_or(0, |f| f as u8 + 1));
            put_number(&mut bytes, u128::from(summary.prior_batches));
            bytes.push(u8::from(summary.transaction_id.is_some()));
            if let Some(transaction) = &summary.transaction_id {
                let signed: i64 = transaction.parse().map_err(|_| "invalid transaction")?;
                put_number(&mut bytes, ((signed << 1) ^ (signed >> 63)) as u64 as u128);
            }
        }
        if bytes.len() > MAX_CAPTURE_BYTES {
            return Err("batch evidence exceeds durable capture bound");
        }
        Ok(bytes)
    }
}
fn put_uuid(bytes: &mut Vec<u8>, id: Option<Uuid>) {
    bytes.push(u8::from(id.is_some()));
    if let Some(id) = id {
        bytes.extend(id.as_bytes());
    }
}
fn put_number(bytes: &mut Vec<u8>, mut value: u128) {
    while value >= 128 {
        bytes.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn number_len(mut value: u128) -> usize {
    let mut length = 1;
    while value >= 128 {
        length += 1;
        value >>= 7;
    }
    length
}

#[cfg(test)]
#[path = "snowflake_batch_tests.rs"]
mod tests;
