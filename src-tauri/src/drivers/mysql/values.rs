use super::*;
use mysql_async::{
    Column,
    consts::{ColumnFlags, ColumnType},
};

pub(super) fn column(index: usize, c: &Column) -> ColumnInfo {
    use ColumnType::*;
    let kind = match c.column_type() {
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_LONGLONG
        | MYSQL_TYPE_INT24 | MYSQL_TYPE_YEAR => ColumnKind::Integer,
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL | MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => {
            ColumnKind::Decimal
        }
        MYSQL_TYPE_DATE
        | MYSQL_TYPE_DATETIME
        | MYSQL_TYPE_TIMESTAMP
        | MYSQL_TYPE_TIME
        | MYSQL_TYPE_NEWDATE
        | MYSQL_TYPE_DATETIME2
        | MYSQL_TYPE_TIMESTAMP2
        | MYSQL_TYPE_TIME2 => ColumnKind::Temporal,
        MYSQL_TYPE_JSON => ColumnKind::Json,
        MYSQL_TYPE_BIT | MYSQL_TYPE_GEOMETRY => ColumnKind::Binary,
        MYSQL_TYPE_VARCHAR
        | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_STRING
        | MYSQL_TYPE_TINY_BLOB
        | MYSQL_TYPE_MEDIUM_BLOB
        | MYSQL_TYPE_LONG_BLOB
        | MYSQL_TYPE_BLOB => {
            if c.character_set() == 63 {
                ColumnKind::Binary
            } else {
                ColumnKind::Text
            }
        }
        _ => ColumnKind::Unknown,
    };
    ColumnInfo {
        id: format!("c{index}"),
        name: c.name_str().into_owned(),
        database_type: {
            let base = match c.column_type() {
                MYSQL_TYPE_TINY => "tinyint",
                MYSQL_TYPE_SHORT => "smallint",
                MYSQL_TYPE_LONG | MYSQL_TYPE_INT24 => "int",
                MYSQL_TYPE_LONGLONG => "bigint",
                MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => "decimal",
                MYSQL_TYPE_FLOAT => "float",
                MYSQL_TYPE_DOUBLE => "double",
                MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => "date",
                MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => "datetime",
                MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => "timestamp",
                MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => "time",
                MYSQL_TYPE_YEAR => "year",
                MYSQL_TYPE_JSON => "json",
                MYSQL_TYPE_BIT => "bit",
                MYSQL_TYPE_GEOMETRY => "geometry",
                _ if kind == ColumnKind::Binary => "binary",
                _ if kind == ColumnKind::Text => "text",
                _ => "unknown",
            };
            if kind == ColumnKind::Integer && c.flags().contains(ColumnFlags::UNSIGNED_FLAG) {
                format!("{base} unsigned")
            } else {
                base.into()
            }
        },
        kind,
        nullable: !c.flags().contains(ColumnFlags::NOT_NULL_FLAG),
        primary_key: false,
        foreign_key: false,
        editable: false,
    }
}

pub(super) fn text_row(row: Row) -> AppResult<TextRow> {
    let metadata = row.columns();
    row.unwrap().into_iter().enumerate().map(|(i,value)| {
        let text=match value {
            Value::NULL=>return Ok(None),
            Value::Bytes(bytes) if binary(&metadata[i])=>format!("0x{}",hex::encode(bytes)),
            Value::Bytes(bytes)=>String::from_utf8(bytes).map_err(|_|AppError::new(ErrorCode::Unsupported,"The server returned non-UTF-8 text. Query HEX(column) to inspect its bytes."))?,
            Value::Int(v)=>v.to_string(),Value::UInt(v)=>v.to_string(),Value::Float(v)=>v.to_string(),Value::Double(v)=>v.to_string(),
            Value::Date(y,m,d,h,min,s,micros)=> {
                if matches!(metadata[i].column_type(),ColumnType::MYSQL_TYPE_DATE|ColumnType::MYSQL_TYPE_NEWDATE) {format!("{y:04}-{m:02}-{d:02}")}
                else { let precision=metadata[i].decimals().min(6) as usize;let fraction=format!("{micros:06}");format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}{}",if precision==0 {String::new()}else{format!(".{}",&fraction[..precision])}) }
            },
            Value::Time(negative,days,h,m,s,micros)=> {
                let precision=metadata[i].decimals().min(6) as usize;let fraction=format!("{micros:06}");
                format!("{}{:02}:{m:02}:{s:02}{}",if negative {"-"}else{""},days*24+u32::from(h),if precision==0{String::new()}else{format!(".{}",&fraction[..precision])})
            },
        };
        Ok(Some(text))
    }).collect()
}

fn binary(c: &Column) -> bool {
    use ColumnType::*;
    matches!(c.column_type(), MYSQL_TYPE_BIT | MYSQL_TYPE_GEOMETRY)
        || (c.character_set() == 63
            && matches!(
                c.column_type(),
                MYSQL_TYPE_VARCHAR
                    | MYSQL_TYPE_VAR_STRING
                    | MYSQL_TYPE_STRING
                    | MYSQL_TYPE_TINY_BLOB
                    | MYSQL_TYPE_MEDIUM_BLOB
                    | MYSQL_TYPE_LONG_BLOB
                    | MYSQL_TYPE_BLOB
            ))
}

pub(super) fn quote(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}
pub(super) fn qualified(table: &TableRef) -> String {
    format!("{}.{}", quote(&table.schema), quote(&table.name))
}
pub(super) fn projection(schema: &TableSchema) -> String {
    schema
        .columns
        .iter()
        .map(|c| quote(&c.name))
        .collect::<Vec<_>>()
        .join(",")
}
pub(super) fn identifier(name: &str) -> AppResult<()> {
    if name.is_empty() || name.chars().count() > 64 || name.contains('\0') {
        Err(AppError::invalid("Invalid MySQL object identifier"))
    } else {
        Ok(())
    }
}

// Use typed binds for keys. Implicit string-to-numeric comparisons can round
// large BIGINT/DECIMAL keys and skip rows above JavaScript's exact integer range.
pub(super) fn cast(c: &ColumnInfo) -> Option<String> {
    let t = c.database_type.to_ascii_lowercase();
    let base = t.split(['(', ' ']).next()?;
    Some(match base {
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "year" => {
            if t.contains("unsigned") {
                "UNSIGNED"
            } else {
                "SIGNED"
            }
            .into()
        }
        "decimal" | "numeric" => {
            let dims = t.strip_prefix(&format!("{base}("))?.split(')').next()?;
            let (p, s) = dims.split_once(',')?;
            let p = p.parse::<u8>().ok()?;
            let s = s.parse::<u8>().ok()?;
            if p == 0 || p > 65 || s > 38 || s > p {
                return None;
            }
            format!("DECIMAL({p},{s})")
        }
        "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext" | "json" | "enum"
        | "set" => "CHAR CHARACTER SET utf8mb4".into(),
        "date" => "DATE".into(),
        "datetime" | "timestamp" => "DATETIME(6)".into(),
        "time" => "TIME(6)".into(),
        // Float binds are deliberately excluded from PKs and grid editing.
        _ => return None,
    })
}
pub(super) fn key_predicate(
    schema: &TableSchema,
    original: &TextRow,
) -> AppResult<(String, Vec<Option<String>>)> {
    if schema.primary_key.is_empty() || original.len() != schema.columns.len() {
        return Err(AppError::invalid(
            "A complete row with a supported primary key is required",
        ));
    }
    let mut parts = Vec::new();
    let mut values = Vec::new();
    for key in &schema.primary_key {
        let i = schema
            .columns
            .iter()
            .position(|c| &c.name == key)
            .ok_or_else(|| AppError::invalid("Missing primary key column"))?;
        let cast = cast(&schema.columns[i])
            .ok_or_else(|| AppError::new(ErrorCode::Unsupported, "Unsupported primary key type"))?;
        if original[i].is_none() {
            return Err(AppError::invalid("A primary key cannot be NULL"));
        }
        parts.push(format!("{}=CAST(? AS {cast})", quote(key)));
        values.push(original[i].clone());
    }
    Ok((parts.join(" AND "), values))
}
pub fn update_sql(
    schema: &TableSchema,
    row: &RowMutation,
) -> AppResult<(String, Vec<Option<String>>)> {
    if !schema.editable || row.changes.is_empty() {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "This table cannot be edited safely",
        ));
    }
    let mut parts = Vec::new();
    let mut values = Vec::new();
    for (i, value) in &row.changes {
        let c = schema
            .columns
            .get(*i)
            .ok_or_else(|| AppError::invalid("Invalid edited column"))?;
        if !c.editable || c.primary_key {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This column is read-only",
            ));
        }
        validate(c, value.as_deref())?;
        let t = cast(c).ok_or_else(|| {
            AppError::new(ErrorCode::Unsupported, "This data type cannot be edited")
        })?;
        parts.push(format!("{}=CAST(? AS {t})", quote(&c.name)));
        values.push(value.clone());
    }
    let (predicate, keys) = key_predicate(schema, &row.original)?;
    values.extend(keys);
    Ok((
        format!(
            "UPDATE {} SET {} WHERE {predicate}",
            qualified(&schema.table),
            parts.join(", ")
        ),
        values,
    ))
}

fn validate(column: &ColumnInfo, value: Option<&str>) -> AppResult<()> {
    let Some(value) = value else {
        return if column.nullable {
            Ok(())
        } else {
            Err(AppError::invalid("This column does not allow NULL"))
        };
    };
    let value = value.trim();
    match column.kind {
        ColumnKind::Integer => {
            let n = value.parse::<i128>().map_err(|_| {
                AppError::invalid("Enter a whole integer without fractional or exponent notation")
            })?;
            let t = column.database_type.to_ascii_lowercase();
            let base = t.split(['(', ' ']).next().unwrap_or("");
            let bits = match base {
                "tinyint" => 8,
                "smallint" | "year" => 16,
                "mediumint" => 24,
                "int" | "integer" => 32,
                _ => 64,
            };
            let (min, max) = if t.contains("unsigned") {
                (0, (1i128 << bits) - 1)
            } else {
                (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
            };
            if n < min || n > max {
                return Err(AppError::invalid(
                    "This integer is outside the column's range",
                ));
            }
        }
        ColumnKind::Decimal => {
            let cast =
                cast(column).ok_or_else(|| AppError::invalid("Unsupported decimal column"))?;
            let dims = cast
                .strip_prefix("DECIMAL(")
                .and_then(|s| s.strip_suffix(')'))
                .ok_or_else(|| AppError::invalid("Unsupported decimal column"))?;
            let (precision, scale) = dims.split_once(',').unwrap();
            let precision = precision.parse::<usize>().unwrap();
            let scale = scale.parse::<usize>().unwrap();
            let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value);
            let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
            if (whole.is_empty() && fraction.is_empty())
                || !whole.bytes().all(|b| b.is_ascii_digit())
                || !fraction.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(AppError::invalid(
                    "Enter a fixed-point decimal without exponent notation",
                ));
            }
            if whole.trim_start_matches('0').len() > precision - scale
                || fraction.trim_end_matches('0').len() > scale
            {
                return Err(AppError::invalid(
                    "This decimal would lose precision in the target column",
                ));
            }
            if column.database_type.contains("unsigned") && value.starts_with('-') {
                return Err(AppError::invalid("This decimal column is unsigned"));
            }
        }
        _ => {}
    }
    Ok(())
}
