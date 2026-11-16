//! Dialect-aware AST SQL usage analysis across workspace SQL buffers and saved queries.

use crate::domain::{
    AppError, AppResult, EngineKind, SqlFindUsagesReport, SqlFindUsagesRequest, SqlSpan,
    SqlUsageDocument, SqlUsageHit,
};
use sqlparser::ast::{
    AssignmentTarget, Expr, Ident, ObjectName, Query, Select, SetExpr, Statement, TableFactor,
    TableObject, TableWithJoins, Visit, Visitor,
};
use sqlparser::dialect::{Dialect, MySqlDialect, PostgreSqlDialect, SnowflakeDialect};
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Location, Span, Token, Tokenizer};
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

pub const MAX_USAGE_DOCUMENTS: usize = 200;
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
pub const MAX_USAGE_HITS: usize = 250;
const MAX_LINE_PREVIEW_CHARS: usize = 240;

pub fn find_sql_usages(request: &SqlFindUsagesRequest) -> AppResult<SqlFindUsagesReport> {
    let target_schema = request.schema.trim();
    let target_object = request.object_name.trim();
    if target_object.is_empty() {
        return Err(AppError::invalid(
            "Provide a target object name for SQL usage search",
        ));
    }
    if request.engine == EngineKind::Redis {
        return Ok(SqlFindUsagesReport {
            scanned_documents: 0,
            skipped_statements: 0,
            truncated: false,
            hits: Vec::new(),
        });
    }

    let target_column = request
        .column_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let default_schema = request
        .default_schema
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(match request.engine {
            EngineKind::Postgres => Some("public"),
            _ => None,
        });
    let active_database = request
        .database
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let dialect = dialect_for_engine(request.engine);
    let mut scanned_documents = 0usize;
    let mut skipped_statements = 0usize;
    let mut truncated = request.documents.len() > MAX_USAGE_DOCUMENTS;
    let mut hits = Vec::new();

    for doc in request.documents.iter().take(MAX_USAGE_DOCUMENTS) {
        if doc.sql.trim().is_empty() {
            continue;
        }
        scanned_documents += 1;
        let sql_slice = if doc.sql.len() > MAX_DOCUMENT_BYTES {
            truncated = true;
            let mut end = MAX_DOCUMENT_BYTES;
            while end > 0 && !doc.sql.is_char_boundary(end) {
                end -= 1;
            }
            &doc.sql[..end]
        } else {
            doc.sql.as_str()
        };

        let index = LineIndex::new(sql_slice);
        let parsed_units =
            parse_document_statements(dialect.as_ref(), sql_slice, &mut skipped_statements);

        for (stmt, byte_offset) in parsed_units {
            let mut collector = UsageCollector {
                engine: request.engine,
                default_schema,
                active_database,
                target_schema,
                target_object,
                target_column,
                doc,
                sql_slice,
                index: &index,
                stmt_byte_offset: byte_offset,
                role: statement_role(&stmt),
                cte_stack: Vec::new(),
                select_scopes: Vec::new(),
                seen_spans: HashSet::new(),
                hits: &mut hits,
                truncated: &mut truncated,
            };
            collector.analyze_statement(&stmt);
            if *collector.truncated {
                break;
            }
        }
        if hits.len() >= MAX_USAGE_HITS {
            truncated = true;
            break;
        }
    }

    Ok(SqlFindUsagesReport {
        scanned_documents,
        skipped_statements,
        truncated,
        hits,
    })
}

fn dialect_for_engine(engine: EngineKind) -> Box<dyn Dialect> {
    match engine {
        EngineKind::Postgres => Box::new(PostgreSqlDialect {}),
        EngineKind::Mysql => Box::new(MySqlDialect {}),
        EngineKind::Snowflake => Box::new(SnowflakeDialect {}),
        EngineKind::Redis => Box::new(PostgreSqlDialect {}),
    }
}

fn statement_role(stmt: &Statement) -> &'static str {
    match stmt {
        Statement::Query(_) => "SELECT",
        Statement::Insert(_) => "INSERT",
        Statement::Update(_) => "UPDATE",
        Statement::Delete(_) => "DELETE",
        Statement::Merge { .. } => "MERGE",
        Statement::CreateView(_) => "VIEW",
        Statement::CreateTable(_)
        | Statement::AlterTable(_)
        | Statement::Drop { .. }
        | Statement::Truncate(_) => "DDL",
        _ => "SQL",
    }
}

fn normalize_ident(engine: EngineKind, ident: &Ident) -> String {
    if ident.quote_style.is_some() {
        return match engine {
            EngineKind::Mysql => ident.value.to_ascii_lowercase(),
            EngineKind::Postgres | EngineKind::Snowflake | EngineKind::Redis => ident.value.clone(),
        };
    }
    match engine {
        EngineKind::Postgres | EngineKind::Mysql | EngineKind::Redis => {
            ident.value.to_ascii_lowercase()
        }
        EngineKind::Snowflake => ident.value.to_ascii_uppercase(),
    }
}

fn normalize_catalog_name(engine: EngineKind, raw: &str) -> String {
    match engine {
        EngineKind::Mysql => raw.to_ascii_lowercase(),
        EngineKind::Postgres | EngineKind::Snowflake | EngineKind::Redis => raw.to_owned(),
    }
}

fn ident_matches_catalog(engine: EngineKind, ident: &Ident, catalog_name: &str) -> bool {
    if ident.quote_style.is_some() {
        return match engine {
            EngineKind::Mysql => ident.value.eq_ignore_ascii_case(catalog_name),
            EngineKind::Postgres | EngineKind::Snowflake | EngineKind::Redis => {
                ident.value == catalog_name
            }
        };
    }
    match engine {
        EngineKind::Postgres | EngineKind::Redis => {
            ident.value.to_ascii_lowercase() == catalog_name
        }
        EngineKind::Snowflake => ident.value.to_ascii_uppercase() == catalog_name,
        EngineKind::Mysql => ident.value.eq_ignore_ascii_case(catalog_name),
    }
}

fn parse_document_statements(
    dialect: &dyn Dialect,
    sql: &str,
    skipped_statements: &mut usize,
) -> Vec<(Statement, usize)> {
    if let Ok(stmts) = Parser::new(dialect)
        .with_recursion_limit(32)
        .try_with_sql(sql)
        .and_then(|mut p| p.parse_statements())
    {
        return stmts.into_iter().map(|s| (s, 0)).collect();
    }

    let Ok(ranges) = split_statement_ranges(dialect, sql) else {
        *skipped_statements += 1;
        return Vec::new();
    };
    let mut out = Vec::new();
    for (start, end) in ranges {
        let sub = &sql[start..end];
        match Parser::new(dialect)
            .with_recursion_limit(32)
            .try_with_sql(sub)
            .and_then(|mut p| p.parse_statements())
        {
            Ok(stmts) => {
                for stmt in stmts {
                    out.push((stmt, start));
                }
            }
            Err(_) => {
                *skipped_statements += 1;
            }
        }
    }
    out
}

fn split_statement_ranges(dialect: &dyn Dialect, sql: &str) -> Result<Vec<(usize, usize)>, ()> {
    let tokens = Tokenizer::new(dialect, sql)
        .tokenize_with_location()
        .map_err(|_| ())?;
    let index = LineIndex::new(sql);
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut has_tokens = false;
    for tok in tokens {
        match tok.token {
            Token::SemiColon => {
                let end = index
                    .location_to_byte(sql, tok.span.start)
                    .unwrap_or(sql.len());
                if has_tokens && start < end && !sql[start..end].trim().is_empty() {
                    parts.push((start, end));
                }
                start = (end + 1).min(sql.len());
                has_tokens = false;
            }
            Token::Whitespace(_) | Token::EOF => {}
            _ => has_tokens = true,
        }
    }
    if has_tokens && start < sql.len() && !sql[start..].trim().is_empty() {
        parts.push((start, sql.len()));
    }
    Ok(parts)
}

struct LineIndex {
    line_byte_starts: Vec<usize>,
    line_utf16_starts: Vec<usize>,
}

impl LineIndex {
    fn new(sql: &str) -> Self {
        let mut line_byte_starts = vec![0usize];
        let mut line_utf16_starts = vec![0usize];
        let mut utf16 = 0usize;
        for (byte_idx, ch) in sql.char_indices() {
            utf16 += ch.len_utf16();
            if ch == '\n' {
                line_byte_starts.push(byte_idx + 1);
                line_utf16_starts.push(utf16);
            }
        }
        Self {
            line_byte_starts,
            line_utf16_starts,
        }
    }

    fn location_to_byte(&self, sql: &str, loc: Location) -> Option<usize> {
        if loc.line == 0 || loc.column == 0 {
            return None;
        }
        let line_idx = (loc.line as usize).checked_sub(1)?;
        let line_start = *self.line_byte_starts.get(line_idx)?;
        let line_end = self
            .line_byte_starts
            .get(line_idx + 1)
            .copied()
            .unwrap_or(sql.len());
        let line_slice = &sql[line_start..line_end];
        let col_idx = (loc.column as usize).checked_sub(1)?;
        if col_idx == 0 {
            return Some(line_start);
        }
        let mut count = 0usize;
        for (off, _) in line_slice.char_indices() {
            if count == col_idx {
                return Some(line_start + off);
            }
            count += 1;
        }
        if count == col_idx {
            Some(line_end)
        } else {
            None
        }
    }

    fn span_in_full_doc(
        &self,
        full_sql: &str,
        stmt_byte_offset: usize,
        local_span: Span,
    ) -> Option<(SqlSpan, String)> {
        let sub_sql = &full_sql[stmt_byte_offset..];
        let (start_byte, end_byte) = if stmt_byte_offset == 0 {
            let s = self.location_to_byte(full_sql, local_span.start)?;
            let e = self.location_to_byte(full_sql, local_span.end).unwrap_or(s);
            (s, e.max(s))
        } else {
            let sub_index = LineIndex::new(sub_sql);
            let s = stmt_byte_offset + sub_index.location_to_byte(sub_sql, local_span.start)?;
            let e = stmt_byte_offset
                + sub_index
                    .location_to_byte(sub_sql, local_span.end)
                    .unwrap_or(0);
            (s, e.max(s))
        };

        let line_idx = match self.line_byte_starts.binary_search(&start_byte) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.line_byte_starts[line_idx];
        let line_utf16_base = self.line_utf16_starts[line_idx];
        let prefix = &full_sql[line_start..start_byte];
        let col = (prefix.chars().count() as u32) + 1;
        let start_utf16 = line_utf16_base + prefix.encode_utf16().count();
        let token_utf16: usize = full_sql[start_byte..end_byte]
            .chars()
            .map(char::len_utf16)
            .sum();
        let end_utf16 = start_utf16 + token_utf16;

        let line_end = self
            .line_byte_starts
            .get(line_idx + 1)
            .copied()
            .unwrap_or(full_sql.len());
        let raw_line = full_sql[line_start..line_end]
            .trim_end_matches(['\r', '\n'])
            .trim();
        let line_text: String = if raw_line.chars().count() > MAX_LINE_PREVIEW_CHARS {
            let mut s: String = raw_line.chars().take(MAX_LINE_PREVIEW_CHARS).collect();
            s.push('…');
            s
        } else {
            raw_line.to_owned()
        };

        Some((
            SqlSpan {
                start_utf16,
                end_utf16,
                line: (line_idx as u32) + 1,
                column: col,
            },
            line_text,
        ))
    }
}

#[derive(Default, Clone)]
struct SelectScope {
    /// Normalized alias/table key -> whether it resolves to the target `(schema, object_name)`.
    aliases: HashMap<String, bool>,
    total_relations: usize,
    target_relations: usize,
}

struct UsageCollector<'a> {
    engine: EngineKind,
    default_schema: Option<&'a str>,
    active_database: Option<&'a str>,
    target_schema: &'a str,
    target_object: &'a str,
    target_column: Option<&'a str>,
    doc: &'a SqlUsageDocument,
    sql_slice: &'a str,
    index: &'a LineIndex,
    stmt_byte_offset: usize,
    role: &'static str,
    cte_stack: Vec<HashSet<String>>,
    select_scopes: Vec<SelectScope>,
    seen_spans: HashSet<(usize, usize)>,
    hits: &'a mut Vec<SqlUsageHit>,
    truncated: &'a mut bool,
}

impl<'a> UsageCollector<'a> {
    fn is_cte_in_scope(&self, ident: &Ident) -> bool {
        let key = normalize_ident(self.engine, ident);
        self.cte_stack.iter().rev().any(|set| set.contains(&key))
    }

    fn schema_matches_default(&self) -> bool {
        if self.target_schema.is_empty() {
            return true;
        }
        let Some(def) = self.default_schema else {
            return false;
        };
        normalize_catalog_name(self.engine, def)
            == normalize_catalog_name(self.engine, self.target_schema)
    }

    /// If `name` resolves to `(target_schema, target_object)` (and is not shadowed by a CTE),
    /// returns the `Ident` of the object name component.
    fn match_object_name<'b>(&self, name: &'b ObjectName) -> Option<&'b Ident> {
        let mut idents = Vec::with_capacity(name.0.len());
        for part in &name.0 {
            idents.push(part.as_ident()?);
        }
        match idents.as_slice() {
            [obj] => {
                if self.is_cte_in_scope(obj) {
                    return None;
                }
                if !self.schema_matches_default() {
                    return None;
                }
                ident_matches_catalog(self.engine, obj, self.target_object).then_some(*obj)
            }
            [sch, obj] => {
                let schema_ok = self.target_schema.is_empty()
                    || ident_matches_catalog(self.engine, sch, self.target_schema);
                (schema_ok && ident_matches_catalog(self.engine, obj, self.target_object))
                    .then_some(*obj)
            }
            [db, sch, obj] if self.engine != EngineKind::Mysql => {
                let db_ok = self
                    .active_database
                    .is_none_or(|active_db| ident_matches_catalog(self.engine, db, active_db));
                let schema_ok = self.target_schema.is_empty()
                    || ident_matches_catalog(self.engine, sch, self.target_schema);
                (db_ok && schema_ok && ident_matches_catalog(self.engine, obj, self.target_object))
                    .then_some(*obj)
            }
            _ => None,
        }
    }

    fn record_ident(&mut self, ident: &Ident, role: &str) {
        if self.hits.len() >= MAX_USAGE_HITS {
            *self.truncated = true;
            return;
        }
        let Some((span, line_text)) =
            self.index
                .span_in_full_doc(self.sql_slice, self.stmt_byte_offset, ident.span)
        else {
            return;
        };
        if !self.seen_spans.insert((span.start_utf16, span.end_utf16)) {
            return;
        }
        self.hits.push(SqlUsageHit {
            document_id: self.doc.id.clone(),
            document_title: self.doc.title.clone(),
            source_kind: self.doc.source_kind,
            saved_query_id: self.doc.saved_query_id.clone(),
            role: role.to_owned(),
            span,
            line_text,
        });
    }

    fn register_table_with_joins(&mut self, twj: &TableWithJoins, scope: &mut SelectScope) {
        self.register_table_factor(&twj.relation, scope);
        for join in &twj.joins {
            self.register_table_factor(&join.relation, scope);
        }
    }

    fn register_table_factor(&mut self, factor: &TableFactor, scope: &mut SelectScope) {
        match factor {
            TableFactor::Table { name, alias, .. } => {
                scope.total_relations += 1;
                let matched_ident = self.match_object_name(name);
                let is_target = matched_ident.is_some();
                if is_target {
                    scope.target_relations += 1;
                    if self.target_column.is_none()
                        && let Some(id) = matched_ident
                    {
                        self.record_ident(id, self.role);
                    }
                }
                if let Some(a) = alias {
                    scope
                        .aliases
                        .insert(normalize_ident(self.engine, &a.name), is_target);
                } else if let Some(last) = name.0.last().and_then(|p| p.as_ident()) {
                    scope
                        .aliases
                        .insert(normalize_ident(self.engine, last), is_target);
                }
            }
            TableFactor::Derived { alias, .. } => {
                scope.total_relations += 1;
                if let Some(a) = alias {
                    scope
                        .aliases
                        .insert(normalize_ident(self.engine, &a.name), false);
                }
            }
            TableFactor::NestedJoin {
                table_with_joins, ..
            } => {
                self.register_table_with_joins(table_with_joins, scope);
            }
            _ => {
                scope.total_relations += 1;
            }
        }
    }

    fn analyze_query(&mut self, query: &Query) {
        let mut local_ctes = HashSet::new();
        if let Some(with) = &query.with {
            if with.recursive {
                for cte in &with.cte_tables {
                    local_ctes.insert(normalize_ident(self.engine, &cte.alias.name));
                }
                self.cte_stack.push(local_ctes.clone());
                for cte in &with.cte_tables {
                    self.analyze_query(&cte.query);
                }
                self.cte_stack.pop();
            } else {
                for cte in &with.cte_tables {
                    self.cte_stack.push(local_ctes.clone());
                    self.analyze_query(&cte.query);
                    self.cte_stack.pop();
                    local_ctes.insert(normalize_ident(self.engine, &cte.alias.name));
                }
            }
        }
        self.cte_stack.push(local_ctes);
        self.analyze_set_expr(&query.body);
        if let Some(order_by) = &query.order_by {
            let _ = order_by.visit(self);
        }
        if let Some(limit_clause) = &query.limit_clause {
            let _ = limit_clause.visit(self);
        }
        self.cte_stack.pop();
    }

    fn analyze_set_expr(&mut self, body: &SetExpr) {
        match body {
            SetExpr::Select(select) => self.analyze_select(select),
            SetExpr::Query(q) => self.analyze_query(q),
            SetExpr::SetOperation { left, right, .. } => {
                self.analyze_set_expr(left);
                self.analyze_set_expr(right);
            }
            SetExpr::Insert(stmt)
            | SetExpr::Update(stmt)
            | SetExpr::Delete(stmt)
            | SetExpr::Merge(stmt) => self.analyze_statement(stmt),
            _ => {
                let _ = body.visit(self);
            }
        }
    }

    fn analyze_select(&mut self, select: &Select) {
        let mut scope = SelectScope::default();
        for twj in &select.from {
            self.register_table_with_joins(twj, &mut scope);
        }
        self.select_scopes.push(scope);

        // Traverse subqueries inside FROM/JOIN (e.g. Derived subqueries, JOIN ON predicates)
        for twj in &select.from {
            self.visit_table_with_joins_children(twj);
        }
        for item in &select.projection {
            let _ = item.visit(self);
        }
        if let Some(prewhere) = &select.prewhere {
            let _ = prewhere.visit(self);
        }
        if let Some(selection) = &select.selection {
            let _ = selection.visit(self);
        }
        let _ = select.group_by.visit(self);
        if let Some(having) = &select.having {
            let _ = having.visit(self);
        }
        if let Some(qualify) = &select.qualify {
            let _ = qualify.visit(self);
        }
        for named_window in &select.named_window {
            let _ = named_window.visit(self);
        }

        self.select_scopes.pop();
    }

    fn visit_table_with_joins_children(&mut self, twj: &TableWithJoins) {
        self.visit_table_factor_children(&twj.relation);
        for join in &twj.joins {
            self.visit_table_factor_children(&join.relation);
            let _ = join.join_operator.visit(self);
        }
    }

    fn visit_table_factor_children(&mut self, factor: &TableFactor) {
        match factor {
            TableFactor::Derived { subquery, .. } => self.analyze_query(subquery),
            TableFactor::NestedJoin {
                table_with_joins, ..
            } => self.visit_table_with_joins_children(table_with_joins),
            TableFactor::Table { args, .. } => {
                if let Some(a) = args {
                    let _ = a.visit(self);
                }
            }
            _ => {
                let _ = factor.visit(self);
            }
        }
    }

    fn analyze_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Query(query) => self.analyze_query(query),
            Statement::Insert(ins) => {
                let mut scope = SelectScope::default();
                if let TableObject::TableName(table_name) = &ins.table {
                    scope.total_relations = 1;
                    if let Some(id) = self.match_object_name(table_name) {
                        scope.target_relations = 1;
                        let alias_ident = ins.table_alias.as_ref().map(|a| &a.alias).unwrap_or(id);
                        scope
                            .aliases
                            .insert(normalize_ident(self.engine, alias_ident), true);
                        if let Some(target_col) = self.target_column {
                            for col in &ins.columns {
                                if let Some(col_ident) = col.0.last().and_then(|p| p.as_ident())
                                    && ident_matches_catalog(self.engine, col_ident, target_col)
                                {
                                    self.record_ident(col_ident, "INSERT");
                                }
                            }
                        } else {
                            self.record_ident(id, "INSERT");
                        }
                    }
                }
                if let Some(src) = &ins.source {
                    self.analyze_query(src);
                }
                self.select_scopes.push(scope);
                if let Some(on) = &ins.on {
                    let _ = on.visit(self);
                }
                if let Some(ret) = &ins.returning {
                    for item in ret {
                        let _ = item.visit(self);
                    }
                }
                self.select_scopes.pop();
            }
            Statement::Update(upd) => {
                let mut scope = SelectScope::default();
                self.register_table_with_joins(&upd.table, &mut scope);
                if let Some(from_kind) = &upd.from {
                    let from_tables = match from_kind {
                        sqlparser::ast::UpdateTableFromKind::BeforeSet(t)
                        | sqlparser::ast::UpdateTableFromKind::AfterSet(t) => t,
                    };
                    for twj in from_tables {
                        self.register_table_with_joins(twj, &mut scope);
                    }
                }
                let target_is_updated = matches!(
                    &upd.table.relation,
                    TableFactor::Table { name, .. } if self.match_object_name(name).is_some()
                );
                if target_is_updated && let Some(target_col) = self.target_column {
                    for assignment in &upd.assignments {
                        if let AssignmentTarget::ColumnName(col_obj) = &assignment.target
                            && let Some(last) = col_obj.0.last().and_then(|p| p.as_ident())
                            && ident_matches_catalog(self.engine, last, target_col)
                        {
                            self.record_ident(last, "UPDATE");
                        }
                    }
                }
                self.select_scopes.push(scope);
                self.visit_table_with_joins_children(&upd.table);
                for assignment in &upd.assignments {
                    let _ = assignment.value.visit(self);
                }
                if let Some(sel) = &upd.selection {
                    let _ = sel.visit(self);
                }
                if let Some(ret) = &upd.returning {
                    for item in ret {
                        let _ = item.visit(self);
                    }
                }
                self.select_scopes.pop();
            }
            Statement::Delete(del) => {
                let mut scope = SelectScope::default();
                let from_tables = match &del.from {
                    sqlparser::ast::FromTable::WithFromKeyword(t)
                    | sqlparser::ast::FromTable::WithoutKeyword(t) => t,
                };
                for twj in from_tables {
                    self.register_table_with_joins(twj, &mut scope);
                }
                if let Some(using) = &del.using {
                    for twj in using {
                        self.register_table_with_joins(twj, &mut scope);
                    }
                }
                self.select_scopes.push(scope);
                for twj in from_tables {
                    self.visit_table_with_joins_children(twj);
                }
                if let Some(sel) = &del.selection {
                    let _ = sel.visit(self);
                }
                if let Some(ret) = &del.returning {
                    for item in ret {
                        let _ = item.visit(self);
                    }
                }
                self.select_scopes.pop();
            }
            Statement::CreateView(cv) => {
                if self.target_column.is_none()
                    && let Some(id) = self.match_object_name(&cv.name)
                {
                    self.record_ident(id, "DDL");
                }
                self.analyze_query(&cv.query);
            }
            other => {
                let _ = other.visit(self);
            }
        }
    }

    fn resolve_alias_in_scopes(&self, alias_ident: &Ident) -> Option<bool> {
        let key = normalize_ident(self.engine, alias_ident);
        for scope in self.select_scopes.iter().rev() {
            if let Some(&is_target) = scope.aliases.get(&key) {
                return Some(is_target);
            }
        }
        None
    }

    fn unambiguous_single_target_in_scope(&self) -> bool {
        let Some(scope) = self.select_scopes.last() else {
            return false;
        };
        scope.total_relations == 1 && scope.target_relations == 1
    }
}

impl Visitor for UsageCollector<'_> {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        // Nested subqueries inside expressions (`WHERE EXISTS (SELECT ...)`, scalar subqueries)
        // get their own lexical CTE + FROM scope; return Continue only after manually analyzing
        // so we don't double-visit. Instead, we cannot skip children in `sqlparser::Visitor`
        // without breaking, so `pre_visit_expr` intercepts `Expr::Subquery` / `Expr::InSubquery` / `Expr::Exists`!
        let _ = query;
        ControlFlow::Continue(())
    }

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        if self.select_scopes.is_empty()
            && self.target_column.is_none()
            && let Some(id) = self.match_object_name(relation)
        {
            self.record_ident(id, self.role);
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Function(func) => {
                if self.target_column.is_none()
                    && let Some(id) = self.match_object_name(&func.name)
                {
                    self.record_ident(id, "FUNCTION");
                }
            }
            Expr::Identifier(ident) => {
                if let Some(target_col) = self.target_column
                    && self.unambiguous_single_target_in_scope()
                    && ident_matches_catalog(self.engine, ident, target_col)
                {
                    self.record_ident(ident, self.role);
                }
            }
            Expr::CompoundIdentifier(parts) => {
                if let Some(target_col) = self.target_column {
                    match parts.as_slice() {
                        [qual, col] if ident_matches_catalog(self.engine, col, target_col) => {
                            if self.resolve_alias_in_scopes(qual) == Some(true) {
                                self.record_ident(col, self.role);
                            }
                        }
                        [sch, tbl, col] if ident_matches_catalog(self.engine, col, target_col) => {
                            let obj = ObjectName::from(vec![sch.clone(), tbl.clone()]);
                            if self.match_object_name(&obj).is_some() {
                                self.record_ident(col, self.role);
                            }
                        }
                        [db, sch, tbl, col]
                            if self.engine != EngineKind::Mysql
                                && ident_matches_catalog(self.engine, col, target_col) =>
                        {
                            let obj = ObjectName::from(vec![db.clone(), sch.clone(), tbl.clone()]);
                            if self.match_object_name(&obj).is_some() {
                                self.record_ident(col, self.role);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SqlUsageSourceKind;

    fn doc(id: &str, title: &str, sql: &str) -> SqlUsageDocument {
        SqlUsageDocument {
            id: id.into(),
            title: title.into(),
            source_kind: SqlUsageSourceKind::OpenTab,
            saved_query_id: None,
            sql: sql.into(),
        }
    }

    #[test]
    fn finds_table_and_routine_usages_while_respecting_cte_shadowing_and_ignoring_comments_strings()
    {
        let report = find_sql_usages(&SqlFindUsagesRequest {
            engine: EngineKind::Postgres,
            connection_id: None,
            database: Some("app_db".into()),
            default_schema: Some("public".into()),
            schema: "public".into(),
            object_name: "orders".into(),
            column_name: None,
            include_saved_queries: false,
            documents: vec![
                doc(
                    "tab-1",
                    "Shadowed CTE & real join",
                    "-- orders in comment\nSELECT 'orders' AS label;\nWITH orders AS (SELECT 1 AS id)\nSELECT * FROM orders;\nSELECT o.id FROM public.orders o;\nUPDATE orders SET status = 'paid' WHERE id = 1;",
                ),
                doc(
                    "tab-2",
                    "Broken draft + valid statement",
                    "SELECT FROM ;\nDELETE FROM app_db.public.orders WHERE id = 9;",
                ),
            ],
        })
        .unwrap();

        assert_eq!(report.scanned_documents, 2);
        assert_eq!(report.skipped_statements, 1);
        assert_eq!(report.hits.len(), 3);
        assert_eq!(report.hits[0].document_id, "tab-1");
        assert_eq!(report.hits[0].role, "SELECT");
        assert_eq!(report.hits[0].span.line, 5);
        assert_eq!(report.hits[1].role, "UPDATE");
        assert_eq!(report.hits[1].span.line, 6);
        assert_eq!(report.hits[2].document_id, "tab-2");
        assert_eq!(report.hits[2].role, "DELETE");
        assert_eq!(report.hits[2].span.line, 2);
    }

    #[test]
    fn finds_column_usages_via_alias_unqualified_single_table_insert_and_update_across_dialects() {
        let pg_col = find_sql_usages(&SqlFindUsagesRequest {
            engine: EngineKind::Postgres,
            connection_id: None,
            database: None,
            default_schema: Some("public".into()),
            schema: "public".into(),
            object_name: "orders".into(),
            column_name: Some("total_cents".into()),
            include_saved_queries: false,
            documents: vec![doc(
                "tab-col",
                "Column refs",
                "SELECT o.total_cents, c.total_cents FROM public.orders o JOIN public.customers c ON c.id = o.customer_id;\nSELECT total_cents FROM orders WHERE total_cents > 0;\nINSERT INTO orders (id, total_cents) VALUES (1, 500);\nUPDATE public.orders SET total_cents = 900 WHERE id = 1;",
            )],
        })
        .unwrap();

        // Line 1: ONLY `o.total_cents` matches (NOT `c.total_cents`!)
        // Line 2: both `SELECT total_cents` and `WHERE total_cents` match
        // Line 3: `INSERT INTO orders (id, total_cents)` matches
        // Line 4: `UPDATE public.orders SET total_cents = 900` matches
        assert_eq!(pg_col.hits.len(), 5);
        assert_eq!(pg_col.hits[0].span.line, 1);
        assert_eq!(pg_col.hits[0].span.column, 10);
        assert_eq!(pg_col.hits[1].span.line, 2);
        assert_eq!(pg_col.hits[2].span.line, 2);
        assert_eq!(pg_col.hits[3].role, "INSERT");
        assert_eq!(pg_col.hits[4].role, "UPDATE");

        // Snowflake uppercase folding vs quoted mixed-case identifier
        let sf = find_sql_usages(&SqlFindUsagesRequest {
            engine: EngineKind::Snowflake,
            connection_id: None,
            database: Some("DB".into()),
            default_schema: Some("PUBLIC".into()),
            schema: "PUBLIC".into(),
            object_name: "ORDERS".into(),
            column_name: Some("NOTES".into()),
            include_saved_queries: false,
            documents: vec![doc(
                "sf-tab",
                "Snowflake query",
                "SELECT o.notes FROM db.public.orders o;\nSELECT o.\"notes\" FROM db.public.orders o;",
            )],
        })
        .unwrap();
        // Unquoted `o.notes` folds to `NOTES` and matches; quoted `o."notes"` is lowercase and does NOT match `NOTES`.
        assert_eq!(sf.hits.len(), 1);
        assert_eq!(sf.hits[0].span.line, 1);
    }
}
