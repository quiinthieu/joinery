use super::*;
use std::collections::BTreeMap;

const KEY_COLUMNS: [&str; 6] = [
    "DATABASE_NAME",
    "SCHEMA_NAME",
    "TABLE_NAME",
    "COLUMN_NAME",
    "KEY_SEQUENCE",
    "CONSTRAINT_NAME",
];
const FOREIGN_COLUMNS: [&str; 11] = [
    "FK_DATABASE_NAME",
    "FK_SCHEMA_NAME",
    "FK_TABLE_NAME",
    "FK_COLUMN_NAME",
    "KEY_SEQUENCE",
    "FK_NAME",
    "PK_DATABASE_NAME",
    "PK_SCHEMA_NAME",
    "PK_TABLE_NAME",
    "PK_COLUMN_NAME",
    "PK_NAME",
];

pub(crate) fn mapping_sql(
    target: &BrowseRelation,
    kind: RelationKeyConstraintKind,
) -> AppResult<String> {
    let table = qualified(target)?;
    if target.traits.dynamic != Some(false) {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Dynamic relation key mappings are unavailable",
        ));
    }
    let (command, columns, name) = match kind {
        RelationKeyConstraintKind::PrimaryKey => {
            ("PRIMARY", KEY_COLUMNS.as_slice(), "CONSTRAINT_NAME")
        }
        RelationKeyConstraintKind::Unique => ("UNIQUE", KEY_COLUMNS.as_slice(), "CONSTRAINT_NAME"),
        RelationKeyConstraintKind::ForeignKey => {
            ("IMPORTED", FOREIGN_COLUMNS.as_slice(), "FK_NAME")
        }
    };
    let projection = columns
        .iter()
        .map(|column| {
            format!(
                "TO_VARCHAR(\"{}\") AS {column}",
                column.to_ascii_lowercase()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    // The pipe retains this SHOW's result, independent of session query history.
    // Lowercase quoted fields are Snowflake's documented SHOW column names.
    Ok(format!(
        "SHOW {command} KEYS IN TABLE {table} ->> SELECT {projection} FROM $1 ORDER BY {name}, KEY_SEQUENCE, {} LIMIT {}",
        columns[3],
        CONSTRAINT_LIMIT + 1
    ))
}

pub(crate) fn unsupported_mappings(
    declarations: &[RelationKeyConstraint],
) -> Vec<RelationConstraintMapping> {
    declarations
        .iter()
        .map(|declaration| mapping(declaration, ConstraintMappingStatus::Unsupported))
        .collect()
}

fn mapping(
    declaration: &RelationKeyConstraint,
    status: ConstraintMappingStatus,
) -> RelationConstraintMapping {
    RelationConstraintMapping {
        catalog: declaration.catalog.clone(),
        schema: declaration.schema.clone(),
        name: declaration.name.clone(),
        kind: declaration.kind,
        status,
        columns: vec![],
        referenced_table: None,
    }
}

struct Group {
    columns: BTreeMap<u32, RelationConstraintColumnMapping>,
    local_names: HashSet<String>,
    referenced_names: HashSet<String>,
    reference: Option<(RelationConstraintTableReference, String)>,
}

pub(crate) fn parse_mappings(
    target: &BrowseRelation,
    declarations: &[RelationKeyConstraint],
    results: [&MetadataResult; 3],
) -> AppResult<Vec<RelationConstraintMapping>> {
    usable(target)?;
    if target.traits.dynamic != Some(false) {
        return Err(malformed());
    }
    let mut output = declarations
        .iter()
        .map(|declaration| mapping(declaration, ConstraintMappingStatus::NotVisible))
        .collect::<Vec<_>>();
    for (kind, result) in [
        RelationKeyConstraintKind::PrimaryKey,
        RelationKeyConstraintKind::Unique,
        RelationKeyConstraintKind::ForeignKey,
    ]
    .into_iter()
    .zip(results)
    {
        let foreign = kind == RelationKeyConstraintKind::ForeignKey;
        exact(
            result,
            if foreign {
                &FOREIGN_COLUMNS
            } else {
                &KEY_COLUMNS
            },
            CONSTRAINT_LIMIT,
            CONSTRAINT_MAX_BYTES,
        )?;
        let mut groups: HashMap<&str, Group> = HashMap::new();
        for row in &result.rows {
            if identifier(&row[0])? != target.database
                || identifier(&row[1])? != target.table.schema
                || identifier(&row[2])? != target.table.name
            {
                return Err(malformed());
            }
            let name = identifier(&row[5])?;
            // SHOW rows never create declarations. A disagreement means drift or
            // ambiguous metadata and invalidates this complete action.
            if !declarations
                .iter()
                .any(|declaration| declaration.name == name && declaration.kind == kind)
            {
                return Err(ambiguous());
            }
            let column_name = identifier(&row[3])?.to_owned();
            let raw_ordinal = required(&row[4])?;
            let ordinal = raw_ordinal.parse::<u32>().map_err(|_| malformed())?;
            if ordinal == 0 || ordinal.to_string() != raw_ordinal {
                return Err(malformed());
            }
            let (reference, referenced_column_name) = if foreign {
                (
                    Some((
                        RelationConstraintTableReference {
                            database: identifier(&row[6])?.to_owned(),
                            table: TableRef {
                                schema: identifier(&row[7])?.to_owned(),
                                name: identifier(&row[8])?.to_owned(),
                            },
                        },
                        identifier(&row[10])?.to_owned(),
                    )),
                    Some(identifier(&row[9])?.to_owned()),
                )
            } else {
                (None, None)
            };
            let group = groups.entry(name).or_insert_with(|| Group {
                columns: BTreeMap::new(),
                local_names: HashSet::new(),
                referenced_names: HashSet::new(),
                reference: reference.clone(),
            });
            if group.reference != reference
                || !group.local_names.insert(column_name.clone())
                || referenced_column_name
                    .as_ref()
                    .is_some_and(|name| !group.referenced_names.insert(name.clone()))
                || group
                    .columns
                    .insert(
                        ordinal,
                        RelationConstraintColumnMapping {
                            ordinal,
                            column_name,
                            referenced_column_name,
                        },
                    )
                    .is_some()
            {
                return Err(ambiguous());
            }
        }
        for (name, group) in groups {
            if group
                .columns
                .keys()
                .copied()
                .ne(1..=group.columns.len() as u32)
            {
                return Err(malformed());
            }
            let index = declarations
                .iter()
                .position(|declaration| declaration.name == name && declaration.kind == kind)
                .ok_or_else(ambiguous)?;
            if let Some(reference) = declarations[index].reference.as_ref() {
                let (table, primary_name) = group.reference.as_ref().ok_or_else(malformed)?;
                if reference.catalog != table.database
                    || reference.schema != table.table.schema
                    || reference.name != *primary_name
                {
                    return Err(ambiguous());
                }
            }
            output[index].status = ConstraintMappingStatus::Available;
            output[index].columns = group.columns.into_values().collect();
            output[index].referenced_table = group.reference.map(|(table, _)| table);
        }
    }
    bounded(&output, CONSTRAINT_MAX_BYTES)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("snowflake_constraint_mappings_tests.rs");
}
