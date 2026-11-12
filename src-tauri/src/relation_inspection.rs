//! Read-only relation browsing contracts. Target IDs and revisions refer to
//! server-held snapshots, not immutable Snowflake object identifiers.

use crate::domain::{ColumnInfo, ColumnKind, CompletionIdentity, TableRef, TextRow};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BrowseCatalog {
    pub revision: String,
    pub identity: CompletionIdentity,
    pub database: String,
    pub relations: Vec<RelationTargetSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationTargetSummary {
    pub target_id: String,
    pub database: String,
    pub table: TableRef,
    pub table_type: String,
    pub traits: RelationTraits,
    pub availability: RelationAvailability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationTraits {
    pub transient: Option<bool>,
    pub temporary: Option<bool>,
    pub iceberg: Option<bool>,
    pub dynamic: Option<bool>,
    pub hybrid: Option<bool>,
    /// READ ONLY property; unrelated to dynamic-table frozen regions.
    pub immutable: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RelationAvailability {
    Available,
    Shadowed,
    Ambiguous,
    UnsupportedKind,
    UnknownTraits,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectRelationRequest {
    pub revision: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefineRelationRequest {
    pub revision: String,
    pub target_id: String,
}

/// Snowflake's current GET_DDL output, not the original creation text or an
/// executable migration. Names remain subject to the catalog's drift checks.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationDefinition {
    pub revision: String,
    pub target_id: String,
    pub database: String,
    pub table: TableRef,
    pub ddl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectRelationConstraintsRequest {
    pub revision: String,
    pub target_id: String,
}

/// Visible declared key constraints, without column mappings or edit authority.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationConstraints {
    pub revision: String,
    pub target_id: String,
    pub database: String,
    pub table: TableRef,
    pub constraints: Vec<RelationKeyConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationKeyConstraint {
    pub catalog: String,
    pub schema: String,
    pub name: String,
    pub kind: RelationKeyConstraintKind,
    pub is_deferrable: bool,
    pub initially_deferred: bool,
    pub enforced: bool,
    pub rely: bool,
    pub comment: Option<String>,
    pub reference: Option<RelationConstraintReference>,
    pub reference_status: ConstraintReferenceStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RelationKeyConstraintKind {
    PrimaryKey,
    Unique,
    ForeignKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintReferenceStatus {
    NotApplicable,
    Available,
    /// No REFERENTIAL_CONSTRAINTS row was returned by the current role.
    NotVisible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationConstraintReference {
    /// Identifies the referenced unique constraint, not its table or columns.
    pub catalog: String,
    pub schema: String,
    pub name: String,
    pub match_option: String,
    pub update_rule: String,
    pub delete_rule: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectRelationConstraintMappingsRequest {
    pub revision: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationConstraintMappings {
    pub revision: String,
    pub target_id: String,
    pub database: String,
    pub table: TableRef,
    pub mappings: Vec<RelationConstraintMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationConstraintMapping {
    pub catalog: String,
    pub schema: String,
    pub name: String,
    pub kind: RelationKeyConstraintKind,
    pub status: ConstraintMappingStatus,
    pub columns: Vec<RelationConstraintColumnMapping>,
    pub referenced_table: Option<RelationConstraintTableReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintMappingStatus {
    Available,
    NotVisible,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationConstraintColumnMapping {
    pub ordinal: u32,
    pub column_name: String,
    pub referenced_column_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationConstraintTableReference {
    pub database: String,
    pub table: TableRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewRelationRequest {
    pub revision: String,
    pub target_id: String,
    pub inspection_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationInspection {
    pub revision: String,
    pub target_id: String,
    pub database: String,
    pub table: TableRef,
    pub columns: Vec<ColumnInspection>,
    pub constraints: ConstraintInspectionStatus,
}

/// No empty key arrays masquerading as an authoritative constraint inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintInspectionStatus {
    NotFetched,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInspection {
    pub name: String,
    pub ordinal: u32,
    pub full_type: String,
    pub data_type: String,
    pub data_type_alias: Option<String>,
    pub kind: InspectedColumnKind,
    pub result_kind: ColumnKind,
    pub nullable: bool,
    /// INFORMATION_SCHEMA.COLUMNS.COLUMN_DEFAULT; not an inferred Iceberg default.
    pub default_expression: Option<String>,
    /// DESCRIBE's default field (the initial default for Iceberg tables).
    pub descriptor_default_expression: Option<String>,
    /// DESCRIBE's independently reported write default, without expression evaluation.
    pub write_default_expression: Option<String>,
    /// Distinguishes an absent descriptor field from a field reporting SQL NULL.
    pub write_default_status: WriteDefaultObservationStatus,
    pub comment: Option<String>,
    // Decimal text avoids lossy conversion through JavaScript Number.
    pub character_maximum_length: Option<String>,
    pub character_octet_length: Option<String>,
    pub numeric_precision: Option<String>,
    pub numeric_precision_radix: Option<String>,
    pub numeric_scale: Option<String>,
    pub identity: ColumnIdentity,
    pub expression: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum WriteDefaultObservationStatus {
    NotReported,
    Available,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum InspectedColumnKind {
    Column,
    VirtualColumn,
    PartitionColumn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ColumnIdentity {
    pub is_identity: Option<bool>,
    pub generation: Option<String>,
    pub start: Option<String>,
    pub increment: Option<String>,
    pub ordered: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelationPreview {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<TextRow>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_reject_sql_fragments_and_unknown_options() {
        for unknown in ["sql", "objectType", "database", "inspectionRevision"] {
            let mut value = serde_json::json!({"revision":"r", "targetId":"opaque"});
            value[unknown] = "caller input".into();
            assert!(serde_json::from_value::<DefineRelationRequest>(value).is_err());
        }
        let definition: DefineRelationRequest = serde_json::from_value(serde_json::json!({
            "revision":"r", "targetId":"opaque"
        }))
        .unwrap();
        assert_eq!(definition.revision, "r");
        assert_eq!(definition.target_id, "opaque");
        for unknown in ["sql", "catalog", "schema", "table", "inspectionRevision"] {
            let mut value = serde_json::json!({"revision":"r", "targetId":"opaque"});
            value[unknown] = "caller input".into();
            assert!(
                serde_json::from_value::<InspectRelationConstraintMappingsRequest>(value.clone())
                    .is_err()
            );
            assert!(serde_json::from_value::<InspectRelationConstraintsRequest>(value).is_err());
        }
        let constraints: InspectRelationConstraintsRequest =
            serde_json::from_value(serde_json::json!({
                "revision":"r", "targetId":"opaque"
            }))
            .unwrap();
        assert_eq!(constraints.target_id, "opaque");
        assert!(
            serde_json::from_value::<PreviewRelationRequest>(serde_json::json!({
                "revision": "r", "targetId": "opaque", "inspectionRevision": "i",
                "orderBy": "secret()"
            }))
            .is_err()
        );
        let request: InspectRelationRequest = serde_json::from_value(serde_json::json!({
            "revision": "r", "targetId": "opaque"
        }))
        .unwrap();
        assert_eq!(request.target_id, "opaque");
    }

    #[test]
    fn unavailable_constraints_and_unknown_traits_are_explicit_on_wire() {
        for (status, wire) in [
            (ConstraintMappingStatus::Available, "available"),
            (ConstraintMappingStatus::NotVisible, "notVisible"),
            (ConstraintMappingStatus::Unsupported, "unsupported"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), wire);
        }
        assert_eq!(
            serde_json::to_value(RelationKeyConstraintKind::PrimaryKey).unwrap(),
            "primaryKey"
        );
        assert_eq!(
            serde_json::to_value(RelationKeyConstraintKind::Unique).unwrap(),
            "unique"
        );
        assert_eq!(
            serde_json::to_value(RelationKeyConstraintKind::ForeignKey).unwrap(),
            "foreignKey"
        );
        assert_eq!(
            serde_json::to_value(ConstraintReferenceStatus::NotApplicable).unwrap(),
            "notApplicable"
        );
        assert_eq!(
            serde_json::to_value(ConstraintReferenceStatus::Available).unwrap(),
            "available"
        );
        assert_eq!(
            serde_json::to_value(ConstraintReferenceStatus::NotVisible).unwrap(),
            "notVisible"
        );
        assert_eq!(
            serde_json::to_value(ConstraintInspectionStatus::NotFetched).unwrap(),
            "notFetched"
        );
        let traits = RelationTraits {
            transient: None,
            temporary: Some(false),
            iceberg: Some(false),
            dynamic: Some(false),
            hybrid: Some(false),
            immutable: Some(false),
        };
        let value = serde_json::to_value(traits).unwrap();
        assert!(value["transient"].is_null());
        assert_eq!(value["temporary"], false);
        assert!(RelationInspection::decl().contains("constraints"));
        assert!(ColumnInspection::decl().contains("defaultExpression"));
        assert!(ColumnInspection::decl().contains("descriptorDefaultExpression"));
        assert!(ColumnInspection::decl().contains("writeDefaultStatus"));
        assert_eq!(
            serde_json::to_value(WriteDefaultObservationStatus::NotReported).unwrap(),
            "notReported"
        );
        assert_eq!(
            serde_json::to_value(WriteDefaultObservationStatus::Available).unwrap(),
            "available"
        );
    }
}
