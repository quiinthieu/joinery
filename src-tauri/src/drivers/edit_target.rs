//! Backend-owned identity. Never accepted from IPC or reconstructed at approval.
use crate::domain::*;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ColumnOrigin {
    pub table_oid: u32,
    pub column_id: i16,
    pub type_oid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PgAttribute {
    pub name: String,
    pub number: i16,
    pub type_oid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum EditIdentity {
    Postgres {
        relation_oid: u32,
        attributes: Vec<PgAttribute>,
    },
}

impl EditIdentity {
    pub fn matches_projection(&self, columns: &[ColumnInfo], origins: &[ColumnOrigin]) -> bool {
        match self {
            Self::Postgres {
                relation_oid,
                attributes,
            } => {
                *relation_oid != 0
                    && columns.len() == origins.len()
                    && columns.iter().zip(origins).all(|(column, origin)| {
                        origin.table_oid == *relation_oid
                            && attributes.iter().any(|a| {
                                a.name == column.name
                                    && a.number == origin.column_id
                                    && a.type_oid == origin.type_oid
                            })
                    })
            }
        }
    }
}

#[derive(Clone)]
pub struct EditTarget {
    pub schema: TableSchema,
    pub identity: Option<EditIdentity>,
}
