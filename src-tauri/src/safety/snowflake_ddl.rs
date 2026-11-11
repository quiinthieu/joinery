//! Pure generated-only admission for the initial reviewed Snowflake table DDL.
//!
//! This module establishes syntax, never permission, object identity, current
//! catalog state, transaction authority, dependency absence, or remote effects.
//! It accepts identifier *values*, not SQL identifier fragments. Every component
//! is quoted without trimming, case folding, or splitting embedded dots. There
//! is deliberately no raw-SQL entry point.

use crate::{
    ddl::{TableColumnDraft, TableObjectAction},
    domain::{AppError, AppResult, DatabaseObjectKind, ErrorCode},
};
use sqlparser::{
    ast::{
        AlterTable, AlterTableOperation, ColumnDef, ColumnOption, ColumnOptionDef, DataType, Ident,
        ObjectName, ObjectType, Statement, helpers::attached_token::AttachedToken,
    },
    dialect::SnowflakeDialect,
    parser::Parser,
};
use std::collections::HashSet;

pub const MAX_COLUMNS: usize = 256;
pub const MAX_IDENTIFIER_CHARS: usize = 255;
pub const MAX_TYPE_BYTES: usize = 64;
pub const MAX_CANONICAL_BYTES: usize = 256 * 1024;
// Deliberately conservative local limits, not engine-capacity claims.
pub const MAX_VARCHAR_LENGTH: u64 = 16_777_216;
pub const MAX_BINARY_LENGTH: u64 = 8_388_608;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    CreateTable,
    AddColumn,
    DropTable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canonical {
    kind: Kind,
    qualified_target: [String; 3],
    sql: String,
}

impl Canonical {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn qualified_target(&self) -> &[String; 3] {
        &self.qualified_target
    }

    pub fn sql(&self) -> &str {
        &self.sql
    }
}

fn unsupported() -> AppError {
    AppError::new(
        ErrorCode::Unsupported,
        "This action is outside the supported generated Snowflake table DDL grammar",
    )
}

fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Generated Snowflake DDL exceeds the local review limits",
    )
}

fn identifier(value: &str) -> AppResult<Ident> {
    if value.len() > MAX_IDENTIFIER_CHARS * 4 || value.chars().count() > MAX_IDENTIFIER_CHARS {
        return Err(limit());
    }
    if value.is_empty()
        || value.chars().all(char::is_whitespace)
        || value.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return Err(unsupported());
    }
    Ok(Ident::with_quote('"', value))
}

fn target(database: &str, schema: &str, table: &str) -> AppResult<(ObjectName, [String; 3])> {
    let components = [database, schema, table];
    let identifiers = components
        .iter()
        .map(|value| identifier(value))
        .collect::<AppResult<Vec<_>>>()?;
    Ok((ObjectName::from(identifiers), components.map(str::to_owned)))
}

/// Parse a small ASCII form grammar before involving the SQL parser. Only
/// these names are accepted; aliases and session-dependent TIMESTAMP are not.
/// Defaults become explicit canonical bounds. Whitespace may surround a type
/// or its numeric arguments, but never joins separate name/digit fragments.
fn scalar_type(input: &str) -> AppResult<DataType> {
    if input.len() > MAX_TYPE_BYTES {
        return Err(limit());
    }
    if !input.is_ascii() || input.bytes().any(|b| b.is_ascii_control()) {
        return Err(unsupported());
    }
    let upper = input.trim().to_ascii_uppercase();
    let (name, args) = if let Some((name, tail)) = upper.split_once('(') {
        let arguments = tail.strip_suffix(')').ok_or_else(unsupported)?;
        if arguments.is_empty() {
            return Err(unsupported());
        }
        let args = arguments
            .split(',')
            .map(|argument| {
                let argument = argument.trim();
                if argument.is_empty() || !argument.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(unsupported());
                }
                argument.parse::<u64>().map_err(|_| unsupported())
            })
            .collect::<AppResult<Vec<_>>>()?;
        (name.trim(), args)
    } else {
        (upper.as_str(), Vec::new())
    };
    let canonical = match (name, args.as_slice()) {
        ("NUMBER", []) => "NUMBER(38,0)".to_owned(),
        ("NUMBER", [precision]) if (1..=38).contains(precision) => {
            format!("NUMBER({precision},0)")
        }
        ("NUMBER", [precision, scale])
            if (1..=38).contains(precision) && *scale <= 37 && scale <= precision =>
        {
            format!("NUMBER({precision},{scale})")
        }
        ("VARCHAR", []) => format!("VARCHAR({MAX_VARCHAR_LENGTH})"),
        ("VARCHAR", [length]) if (1..=MAX_VARCHAR_LENGTH).contains(length) => {
            format!("VARCHAR({length})")
        }
        ("BINARY", []) => format!("BINARY({MAX_BINARY_LENGTH})"),
        ("BINARY", [length]) if (1..=MAX_BINARY_LENGTH).contains(length) => {
            format!("BINARY({length})")
        }
        ("TIME", []) => "TIME(9)".to_owned(),
        ("TIME", [precision]) if *precision <= 9 => format!("TIME({precision})"),
        ("TIMESTAMP_NTZ", []) => "TIMESTAMP_NTZ(9)".to_owned(),
        ("TIMESTAMP_NTZ", [precision]) if *precision <= 9 => {
            format!("TIMESTAMP_NTZ({precision})")
        }
        ("BOOLEAN" | "FLOAT" | "DATE", []) => name.to_owned(),
        _ => return Err(unsupported()),
    };
    // Only the closed canonical spelling above reaches this parser. It is not
    // a fallback for arbitrary data_type strings from a client.
    Parser::new(&SnowflakeDialect {})
        .with_recursion_limit(16)
        .try_with_sql(&canonical)
        .and_then(|mut parser| parser.parse_data_type())
        .map_err(|_| unsupported())
}

fn column(draft: &TableColumnDraft, adding: bool) -> AppResult<ColumnDef> {
    // Some("") is still a supplied forbidden field, not an absent default.
    if draft.default_expression.is_some()
        || draft.auto_increment
        || draft.primary_key
        || (adding && !draft.nullable)
    {
        return Err(unsupported());
    }
    Ok(ColumnDef {
        name: identifier(&draft.name)?,
        data_type: scalar_type(&draft.data_type)?,
        options: if draft.nullable {
            Vec::new()
        } else {
            vec![ColumnOptionDef {
                name: None,
                option: ColumnOption::NotNull,
            }]
        },
    })
}

fn parse_one(sql: &str) -> AppResult<Statement> {
    let statements = Parser::new(&SnowflakeDialect {})
        .with_recursion_limit(32)
        .try_with_sql(sql)
        .and_then(|mut parser| parser.parse_statements())
        .map_err(|_| unsupported())?;
    let mut statements = statements.into_iter();
    let statement = statements.next().ok_or_else(unsupported)?;
    if statements.next().is_some() {
        return Err(unsupported());
    }
    Ok(statement)
}

/// Build exactly one statement from a structured action and an independently
/// observed database identifier value. This checks the database syntax only;
/// the caller must bind it to fresh catalog/session authority before execution.
pub fn compile(action: &TableObjectAction, database: &str) -> AppResult<Canonical> {
    let (kind, qualified_target, statement) = match action {
        TableObjectAction::CreateTable {
            table,
            columns,
            comment,
        } => {
            if comment.is_some() || columns.is_empty() {
                return Err(unsupported());
            }
            if columns.len() > MAX_COLUMNS {
                return Err(limit());
            }
            let (name, target) = target(database, &table.schema, &table.name)?;
            let mut seen = HashSet::new();
            let columns = columns
                .iter()
                .map(|draft| {
                    let column = column(draft, false)?;
                    // All names are quoted: equality is exact and case-sensitive.
                    if !seen.insert(column.name.value.clone()) {
                        return Err(unsupported());
                    }
                    Ok(column)
                })
                .collect::<AppResult<Vec<_>>>()?;
            // A fixed feature-free template supplies every parser-specific
            // default. Replacing only name/columns cannot introduce CTAS,
            // constraints, policies, options, LIKE, CLONE, or OR REPLACE.
            let Statement::CreateTable(mut create) =
                parse_one("CREATE TABLE \"D\".\"S\".\"T\" (\"C\" BOOLEAN)")?
            else {
                return Err(unsupported());
            };
            create.name = name;
            create.columns = columns;
            (Kind::CreateTable, target, Statement::CreateTable(create))
        }
        TableObjectAction::AddColumn {
            table,
            column: draft,
        } => {
            let (name, target) = target(database, &table.schema, &table.name)?;
            let statement = Statement::AlterTable(AlterTable {
                name,
                if_exists: false,
                only: false,
                operations: vec![AlterTableOperation::AddColumn {
                    column_keyword: true,
                    if_not_exists: false,
                    column_def: column(draft, true)?,
                    column_position: None,
                }],
                location: None,
                on_cluster: None,
                table_type: None,
                end_token: AttachedToken::empty(),
            });
            (Kind::AddColumn, target, statement)
        }
        TableObjectAction::DropObject { object, cascade } => {
            if *cascade || object.kind != DatabaseObjectKind::Table || object.identity.is_some() {
                return Err(unsupported());
            }
            let (name, target) = target(database, &object.schema, &object.name)?;
            let statement = Statement::Drop {
                object_type: ObjectType::Table,
                if_exists: false,
                names: vec![name],
                cascade: false,
                // Plain DROP defaults to CASCADE for standard Snowflake
                // tables. RESTRICT is explicit but does not establish that
                // dependent views or other indirect effects are absent.
                restrict: true,
                purge: false,
                temporary: false,
                table: None,
            };
            (Kind::DropTable, target, statement)
        }
        // This is intentionally exhaustive: a new action must make an explicit
        // choice here rather than inherit a permissive default.
        TableObjectAction::AlterColumn { .. } | TableObjectAction::DropColumn { .. } => {
            return Err(unsupported());
        }
    };
    let sql = statement.to_string();
    if sql.len() > MAX_CANONICAL_BYTES {
        return Err(limit());
    }
    // Whole-AST equality checks all modifiers, operations, targets, column
    // options and type arguments. A parser or rendering mismatch fails closed.
    let roundtrip = parse_one(&sql)?;
    if roundtrip != statement || roundtrip.to_string() != sql {
        return Err(unsupported());
    }
    Ok(Canonical {
        kind,
        qualified_target,
        sql,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DatabaseObjectRef, TableRef};

    fn table() -> TableRef {
        TableRef {
            schema: "SCRATCH".into(),
            name: "T".into(),
        }
    }

    fn draft(name: &str, data_type: &str) -> TableColumnDraft {
        TableColumnDraft {
            name: name.into(),
            data_type: data_type.into(),
            nullable: true,
            default_expression: None,
            auto_increment: false,
            primary_key: false,
        }
    }

    fn create(columns: Vec<TableColumnDraft>) -> TableObjectAction {
        TableObjectAction::CreateTable {
            table: table(),
            columns,
            comment: None,
        }
    }

    fn drop_table() -> TableObjectAction {
        TableObjectAction::DropObject {
            object: DatabaseObjectRef {
                schema: "SCRATCH".into(),
                name: "T".into(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
            cascade: false,
        }
    }

    #[test]
    fn generated_ddl_exact_create_add_drop() {
        let mut amount = draft("Amount", "number(12, 2)");
        amount.nullable = false;
        let result = compile(&create(vec![amount, draft("Text", "varchar(80)")]), "DB").unwrap();
        assert_eq!(result.kind(), Kind::CreateTable);
        assert_eq!(result.qualified_target(), &["DB", "SCRATCH", "T"]);
        assert_eq!(
            result.sql(),
            "CREATE TABLE \"DB\".\"SCRATCH\".\"T\" (\"Amount\" NUMBER(12, 2) NOT NULL, \"Text\" VARCHAR(80))"
        );
        let add = TableObjectAction::AddColumn {
            table: table(),
            column: draft("new", "boolean"),
        };
        let result = compile(&add, "DB").unwrap();
        assert_eq!(result.kind(), Kind::AddColumn);
        assert_eq!(
            result.sql(),
            "ALTER TABLE \"DB\".\"SCRATCH\".\"T\" ADD COLUMN \"new\" BOOLEAN"
        );
        let result = compile(&drop_table(), "DB").unwrap();
        assert_eq!(result.kind(), Kind::DropTable);
        assert_eq!(result.sql(), "DROP TABLE \"DB\".\"SCRATCH\".\"T\" RESTRICT");
    }

    #[test]
    fn generated_ddl_identifiers_are_lossless_single_components() {
        let action = TableObjectAction::CreateTable {
            table: TableRef {
                schema: "s.with.dot".into(),
                name: " x\"; DROP TABLE y;-- ".into(),
            },
            columns: vec![draft("雪\".col", "BOOLEAN")],
            comment: None,
        };
        let result = compile(&action, "d\"b").unwrap();
        assert_eq!(
            result.qualified_target(),
            &["d\"b", "s.with.dot", " x\"; DROP TABLE y;-- "]
        );
        assert_eq!(
            result.sql(),
            "CREATE TABLE \"d\"\"b\".\"s.with.dot\".\" x\"\"; DROP TABLE y;-- \" (\"雪\"\".col\" BOOLEAN)"
        );
        assert!(matches!(
            parse_one(result.sql()).unwrap(),
            Statement::CreateTable(_)
        ));
    }

    #[test]
    fn generated_ddl_closed_scalar_types_and_boundaries() {
        for input in [
            "NUMBER",
            "NUMBER(1)",
            "NUMBER(38,37)",
            "VARCHAR",
            "VARCHAR(1)",
            "VARCHAR(16777216)",
            "BINARY",
            "BINARY(1)",
            "BINARY(8388608)",
            "BOOLEAN",
            "FLOAT",
            "DATE",
            "TIME",
            "TIME(0)",
            "TIME(9)",
            "TIMESTAMP_NTZ",
            "TIMESTAMP_NTZ(0)",
            "TIMESTAMP_NTZ(9)",
            " number ( 01 , 0 ) ",
        ] {
            let result = compile(&create(vec![draft("c", input)]), "DB");
            assert!(result.is_ok(), "{input}: {result:?}");
        }
        let result = compile(&create(vec![draft("c", "NUMBER")]), "DB").unwrap();
        assert!(result.sql().contains("NUMBER(38, 0)"));
        let result = compile(&create(vec![draft("c", "TIME")]), "DB").unwrap();
        assert!(result.sql().contains("TIME(9)"));
    }

    #[test]
    fn generated_ddl_rejects_type_expressions_options_aliases_and_limits() {
        for input in [
            "",
            " ",
            "NUMBER(0)",
            "NUMBER(39)",
            "NUMBER(3,4)",
            "NUMBER(38,38)",
            "NUMBER(3,-1)",
            "NUMBER(1,0,0)",
            "NUMBER()",
            "NUMBER((1))",
            "NUMBER(1) trailing",
            "NUMBER(1); DROP TABLE x",
            "NUMBER(1)/*x*/",
            "NUMBER(1)--x",
            "NUMBER(1_0)",
            "NUMBER(+1)",
            "NUMBER(1e1)",
            "NUMBER(1 0)",
            "NUMBER(18446744073709551616)",
            "VARCHAR(0)",
            "VARCHAR(16777217)",
            "VARCHAR(MAX)",
            "BINARY(8388609)",
            "TIME(10)",
            "TIME(1,0)",
            "TIMESTAMP_NTZ(10)",
            "TIMESTAMP",
            "TIMESTAMP_LTZ",
            "TIMESTAMP_TZ",
            "VARIANT",
            "OBJECT",
            "ARRAY",
            "VECTOR(FLOAT,3)",
            "GEOGRAPHY",
            "INT",
            "DECIMAL(5,2)",
            "STRING",
            "TEXT",
            "FLOAT(2)",
            "DOUBLE",
            "BOOL",
            "BOOLEAN NOT NULL",
            "VARCHAR(1) COLLATE 'en'",
            "DATE DEFAULT CURRENT_DATE()",
            "VARCHAR(1) WITH MASKING POLICY p",
            "VARCHAR(1) COMMENT 'x'",
            "NUMBER\n",
            "ＮＵＭＢＥＲ",
        ] {
            assert!(
                compile(&create(vec![draft("c", input)]), "DB").is_err(),
                "{input}"
            );
        }
        assert!(
            compile(
                &create(vec![draft("c", &"N".repeat(MAX_TYPE_BYTES + 1))]),
                "DB"
            )
            .is_err()
        );
    }

    #[test]
    fn generated_ddl_forbidden_draft_fields_are_not_silently_discarded() {
        for mutation in 0..4 {
            let mut column = draft("c", "NUMBER");
            match mutation {
                0 => column.auto_increment = true,
                1 => column.primary_key = true,
                2 => column.default_expression = Some(String::new()),
                _ => column.default_expression = Some("1".into()),
            }
            assert!(compile(&create(vec![column.clone()]), "DB").is_err());
            assert!(
                compile(
                    &TableObjectAction::AddColumn {
                        table: table(),
                        column
                    },
                    "DB"
                )
                .is_err()
            );
        }
        let mut not_null = draft("c", "NUMBER");
        not_null.nullable = false;
        assert!(
            compile(
                &TableObjectAction::AddColumn {
                    table: table(),
                    column: not_null
                },
                "DB"
            )
            .is_err()
        );
        for comment in [String::new(), "comment".into()] {
            let action = TableObjectAction::CreateTable {
                table: table(),
                columns: vec![draft("c", "NUMBER")],
                comment: Some(comment),
            };
            assert!(compile(&action, "DB").is_err());
        }
    }

    #[test]
    fn generated_ddl_duplicate_names_are_exact_quoted_values() {
        assert!(compile(&create(vec![draft("c", "DATE"), draft("c", "DATE")]), "DB").is_err());
        assert!(compile(&create(vec![draft("c", "DATE"), draft("C", "DATE")]), "DB").is_ok());
        assert!(
            compile(
                &create(vec![draft("é", "DATE"), draft("e\u{301}", "DATE")]),
                "DB"
            )
            .is_ok()
        );
        assert!(compile(&create(Vec::new()), "DB").is_err());
        let columns = (0..MAX_COLUMNS)
            .map(|i| draft(&format!("c{i}"), "DATE"))
            .collect::<Vec<_>>();
        assert!(compile(&create(columns.clone()), "DB").is_ok());
        let mut oversized = columns;
        oversized.push(draft("one_more", "DATE"));
        assert!(compile(&create(oversized), "DB").is_err());
    }

    #[test]
    fn generated_ddl_invalid_identifiers_fail_in_every_component() {
        for invalid in [
            "".into(),
            "  ".into(),
            "a\0b".into(),
            "a\nb".into(),
            "a\u{202e}b".into(),
            "a\u{2066}b".into(),
            "a\u{2028}b".into(),
            "x".repeat(256),
            "雪".repeat(256),
        ] {
            assert!(compile(&create(vec![draft("c", "DATE")]), &invalid).is_err());
            for component in 0..3 {
                let mut target = table();
                let mut column = draft("c", "DATE");
                match component {
                    0 => target.schema = invalid.clone(),
                    1 => target.name = invalid.clone(),
                    _ => column.name = invalid.clone(),
                }
                assert!(
                    compile(
                        &TableObjectAction::CreateTable {
                            table: target,
                            columns: vec![column],
                            comment: None
                        },
                        "DB"
                    )
                    .is_err()
                );
            }
        }
        let name = "雪".repeat(MAX_IDENTIFIER_CHARS);
        assert!(compile(&create(vec![draft(&name, "DATE")]), &name).is_ok());
    }

    #[test]
    fn generated_ddl_drop_is_one_plain_table_only() {
        let result = compile(&drop_table(), "DB").unwrap();
        let Statement::Drop {
            restrict, cascade, ..
        } = parse_one(result.sql()).unwrap()
        else {
            panic!("generated DROP must remain a DROP statement");
        };
        assert!(restrict, "DROP must explicitly request RESTRICT");
        assert!(!cascade, "DROP must not request CASCADE");
        for kind in [
            DatabaseObjectKind::PartitionedTable,
            DatabaseObjectKind::View,
            DatabaseObjectKind::MaterializedView,
            DatabaseObjectKind::Function,
            DatabaseObjectKind::Procedure,
            DatabaseObjectKind::Sequence,
            DatabaseObjectKind::Trigger,
            DatabaseObjectKind::Index,
        ] {
            let TableObjectAction::DropObject { mut object, .. } = drop_table() else {
                unreachable!()
            };
            object.kind = kind;
            assert!(
                compile(
                    &TableObjectAction::DropObject {
                        object,
                        cascade: false
                    },
                    "DB"
                )
                .is_err()
            );
        }
        for identity in [None, Some(String::new()), Some("()".into())] {
            let TableObjectAction::DropObject { mut object, .. } = drop_table() else {
                unreachable!()
            };
            object.identity = identity;
            assert!(
                compile(
                    &TableObjectAction::DropObject {
                        object,
                        cascade: true
                    },
                    "DB"
                )
                .is_err()
            );
        }
        let TableObjectAction::DropObject { mut object, .. } = drop_table() else {
            unreachable!()
        };
        object.identity = Some(String::new());
        assert!(
            compile(
                &TableObjectAction::DropObject {
                    object,
                    cascade: false
                },
                "DB"
            )
            .is_err()
        );
    }

    #[test]
    fn generated_ddl_unsupported_actions_never_reach_rendering() {
        let actions = [
            TableObjectAction::DropColumn {
                table: table(),
                column_name: "c".into(),
                cascade: false,
            },
            TableObjectAction::AlterColumn {
                table: table(),
                column_name: "c".into(),
                new_name: None,
                data_type: "DATE".into(),
                nullable: true,
                default_expression: None,
                using_expression: None,
            },
        ];
        for action in actions {
            assert!(compile(&action, "DB").is_err());
        }
    }

    #[test]
    fn generated_ddl_roundtrip_rejects_multiple_statements() {
        assert!(parse_one("DROP TABLE \"D\".\"S\".\"T\"; DROP TABLE \"D\".\"S\".\"U\"").is_err());
        assert!(parse_one("").is_err());
    }

    #[test]
    fn generated_ddl_total_output_is_bounded_independently_of_each_field() {
        let prefix = "\u{1f600}".repeat(MAX_IDENTIFIER_CHARS - 1);
        let columns = (0..MAX_COLUMNS)
            .map(|i| {
                let suffix = char::from_u32(0x10000 + i as u32).unwrap();
                draft(&format!("{prefix}{suffix}"), "VARCHAR")
            })
            .collect();
        let error = compile(&create(columns), "DB").unwrap_err();
        assert_eq!(error.code, ErrorCode::ResultLimitReached);
    }
}
