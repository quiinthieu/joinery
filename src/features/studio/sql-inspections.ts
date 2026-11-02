import type { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import type { TableRef, TableSchema } from "../../ipc/generated";
import { statementTokensAt, tableLookup, type SqlToken } from "./sql-context";
import { identifierValue, readIdentifier } from "./sql-identifiers";
import type { SqlSymbolContext } from "./sql-navigation";

export interface MissingSqlColumn {
  name: string;
  qualifier: string;
  table: TableRef;
  from: number;
  to: number;
  line: number;
  /** Set when the qualifier is a CTE or derived output, not the base table. */
  outputOf?: string;
  /** Correction names for outputOf. Absent on direct-table references. */
  projectedColumns?: readonly string[];
}

export interface SqlColumnInspection {
  tables: TableRef[];
  checked: number;
  issues: MissingSqlColumn[];
  columnsFor(issue: MissingSqlColumn): readonly string[];
  preview(
    state: EditorState,
    issue: MissingSqlColumn,
    column: string,
  ): {
    from: number;
    to: number;
    insert: string;
  };
}

export const columnInspectionLimits = {
  relations: 32,
  columnsPerTable: 2048,
  columns: 8192,
  bytes: 4 * 1024 * 1024,
};
const tableKey = (table: TableRef) => JSON.stringify([table.schema, table.name]);

/**
 * Flat direct-table SELECTs, plus CTEs or a derived table whose body is that
 * shape. A CTE or derived table may join two or more flat SELECT arms with
 * UNION, INTERSECT, or EXCEPT, including ALL or DISTINCT. Its names come from
 * the anchor arm. Only a recursive arm can read that CTE. A full explicit
 * column list renames those outputs, including a star when the list matches
 * that star's width. A select list may be `*`, `alias.*`, or
 * an unaliased `schema.table.*` for one relation. `*` or `alias.*` may also
 * sit beside simple columns, adding those names. A schema-qualified star
 * expands only when FROM writes that same name and the list is only that
 * star. A partial column list withholds. Every arm of a set operation may
 * be a lone `*`, `alias.*`, or unaliased `schema.table.*` of that same
 * relation, including a later arm of a recursive CTE. Each arm may add the
 * same number of simple columns. A star of another relation is accepted
 * when metadata gives every arm the same width. The offered names are the
 * anchor relation's columns, plus the anchor arm's extra names.
 * shortcut: a schema-qualified star beside other items, a different extra
 * count on the same relation, a top-level set operation, more arms than the
 * relation limit, and an arm that is not a flat SELECT withhold. A repeated
 * output name, or a name owned by more than one relation, withholds the
 * inspection. upgrade for those shapes.
 */
export function prepareColumnInspection(
  state: EditorState,
  pos: number,
  context: SqlSymbolContext,
) {
  const unsupported = () =>
    new Error(
      "Column inspection supports a direct-table SELECT with bounded JOINs, without subqueries.",
    );
  if (context.engine !== "postgres" && context.engine !== "mysql") throw unsupported();
  const statement = statementTokensAt(state, pos);
  if (!statement) throw new Error("Statement is incomplete or exceeds the inspection limit.");
  const clean = (tokens: readonly SqlToken[]) => tokens.filter((t) => !t.kind.endsWith("Comment"));
  const tokens = clean(statement.tokens);
  const nameToken = (token?: SqlToken) =>
    !!token && ["Identifier", "QuotedIdentifier"].includes(token.kind);
  const checkName = (token: SqlToken) => {
    const name = readIdentifier(token.text).value;
    if (
      (context.engine === "postgres" && new TextEncoder().encode(name).length > 63) ||
      (context.engine === "mysql" && [...name].length > 64)
    )
      throw unsupported();
  };
  const word = (token?: SqlToken) => (token?.kind === "Keyword" ? token.text.toUpperCase() : "");
  const nameLike = (token?: SqlToken) =>
    !!token &&
    (nameToken(token) ||
      (["Keyword", "Type", "Builtin"].includes(token.kind) &&
        /^[A-Za-z_][A-Za-z0-9_$]*$/.test(token.text)));
  const columnKey = (name: string) => (context.engine === "mysql" ? name.toLowerCase() : name);
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (!tree) throw unsupported();
  let invalid = false;
  tree.iterate({
    from: tokens[0].from,
    to: tokens.at(-1)!.to,
    enter(node) {
      if (
        node.type.isError &&
        !(node.from === node.to && state.sliceDoc(node.from - 1, node.from + 1) === ".*")
      )
        invalid = true;
    },
  });
  if (invalid) throw unsupported();
  const lookup = tableLookup(
    [...context.tables],
    context.engine,
    context.database,
    undefined,
    undefined,
    context.unqualifiedTables,
    context.caseSensitiveTableNames,
  );
  const key = (raw: string) => {
    const value = identifierValue(readIdentifier(raw), context.engine);
    return context.engine === "mysql" && context.caseSensitiveTableNames === false
      ? value.toLowerCase()
      : value;
  };
  interface QueryOutput {
    name: string;
    table: TableRef;
    source: string;
  }
  interface Binding {
    key: string;
    /** Alias or table text, used when an unqualified name has one owner. */
    label: string;
    table: TableRef;
    outputOf?: string;
    outputs?: QueryOutput[];
    /** Every catalog column of this table. Names are filled when metadata arrives. */
    starTable?: TableRef;
    /** Explicit CTE names. A star uses them when the list matches its width. */
    columnList?: readonly string[];
    /** Schema key when FROM wrote schema.table with no alias. */
    writtenSchema?: string;
  }
  interface Analysis {
    references: MissingSqlColumn[];
    unqualifiedOwners: Map<MissingSqlColumn, Binding[]>;
    proofs: QueryOutput[];
    tables: TableRef[];
    bindings: Binding[];
  }
  type Scope = Binding[];
  const clauses = new Set(["WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FOR", "FETCH"]);
  const joinModifiers = new Set(["INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS", "NATURAL"]);
  const joins = new Set([
    "",
    "INNER",
    "LEFT",
    "LEFT OUTER",
    "RIGHT",
    "RIGHT OUTER",
    "FULL",
    "FULL OUTER",
    "CROSS",
    "NATURAL",
    "NATURAL INNER",
    "NATURAL LEFT",
    "NATURAL LEFT OUTER",
    "NATURAL RIGHT",
    "NATURAL RIGHT OUTER",
    "NATURAL FULL",
    "NATURAL FULL OUTER",
  ]);
  let relations = 0;
  let visited = 0;
  const starRefs = new Set<MissingSqlColumn>();
  const starExtras = new Map<MissingSqlColumn, readonly string[]>();
  const starRenames = new Map<MissingSqlColumn, readonly string[]>();
  type SetArm =
    { kind: "bare"; starTable: TableRef; extras: number } | { kind: "named"; count: number };
  const setWidths: SetArm[][] = [];
  const rememberStar = (ref: MissingSqlColumn, binding: Binding) => {
    if (!binding.starTable) return;
    starRefs.add(ref);
    if (binding.outputs?.length)
      starExtras.set(
        ref,
        binding.outputs.map((output) => output.name),
      );
    if (binding.columnList) starRenames.set(ref, binding.columnList);
  };
  const projectedFields = (binding: Binding) => {
    if (!binding.outputOf) return {};
    if (binding.starTable) return { outputOf: binding.outputOf };
    return {
      outputOf: binding.outputOf,
      projectedColumns: binding.outputs!.map((output) => output.name),
    };
  };
  function selectItems(select: readonly SqlToken[]): SqlToken[][] {
    const items: SqlToken[][] = [];
    let start = 0;
    for (let i = 0; i <= select.length; i++) {
      if (i < select.length && select[i].text !== ",") continue;
      const item = select.slice(start, i);
      start = i + 1;
      if (!item.length) throw unsupported();
      items.push(item);
    }
    return items;
  }
  function projectItems(items: readonly SqlToken[][], scope: Scope): QueryOutput[] {
    if (items.length > 2048) throw unsupported();
    const outputs: QueryOutput[] = [];
    for (const raw of items) {
      const item = [...raw];
      let alias: SqlToken | undefined;
      if (item.length >= 3 && word(item.at(-2)) === "AS" && nameLike(item.at(-1))) {
        alias = item.at(-1);
        item.splice(-2, 2);
      } else if (item.length >= 2 && nameLike(item.at(-1)) && item.at(-2)?.text !== ".")
        alias = item.pop();
      if (alias) checkName(alias);
      if (!item.length || item.length > 3 || item.length % 2 === 0) throw unsupported();
      for (let j = 0; j < item.length; j++)
        if (j % 2 === 0 ? !nameLike(item[j]) : item[j].text !== ".") throw unsupported();
      const columnTok = item.at(-1)!;
      checkName(columnTok);
      const written = identifierValue(readIdentifier(columnTok.text), context.engine);
      let sourceTable: TableRef;
      let source = written;
      if (item.length === 1) {
        const bases = scope.filter((binding) => !binding.outputs || binding.starTable);
        if (bases.length !== 1) throw unsupported();
        const owner = bases[0];
        const found = owner.outputs?.filter(
          (output) => columnKey(output.name) === columnKey(written),
        );
        if (found && found.length > 1) throw unsupported();
        if (found?.length === 1) {
          sourceTable = found[0].table;
          source = found[0].source;
        } else if (owner.starTable) sourceTable = owner.starTable;
        else if (owner.outputs) throw unsupported();
        else sourceTable = owner.table;
      } else {
        checkName(item[0]);
        const owners = scope.filter((binding) => binding.key === key(item[0].text));
        if (owners.length !== 1) throw unsupported();
        const owner = owners[0];
        const found = owner.outputs?.filter(
          (output) => columnKey(output.name) === columnKey(written),
        );
        if (found && found.length > 1) throw unsupported();
        if (owner.outputs && !owner.starTable) {
          if (found?.length !== 1) throw unsupported();
          sourceTable = found[0].table;
          source = found[0].source;
        } else if (found?.length === 1) {
          sourceTable = found[0].table;
          source = found[0].source;
        } else if (owner.starTable) sourceTable = owner.starTable;
        else sourceTable = owner.table;
      }
      outputs.push({
        name: identifierValue(readIdentifier((alias ?? columnTok).text), context.engine),
        table: sourceTable,
        source,
      });
      if (outputs.length > 256) throw unsupported();
    }
    return outputs;
  }
  function project(list: readonly SqlToken[], scope: Scope): QueryOutput[] {
    if (word(list[0]) !== "SELECT") throw unsupported();
    let at = 1;
    if (word(list[1]) === "DISTINCT" || word(list[1]) === "ALL") at = 2;
    const from = list.findIndex((token) => word(token) === "FROM");
    if (from < at) throw unsupported();
    const select = list.slice(at, from);
    if (!select.length || select.length > 2048) throw unsupported();
    const outputs = projectItems(selectItems(select), scope);
    if (
      !outputs.length ||
      new Set(outputs.map((output) => columnKey(output.name))).size !== outputs.length
    )
      throw unsupported();
    return outputs;
  }
  function analyze(
    tokens: readonly SqlToken[],
    depth: number,
    ctes: readonly Binding[] = [],
  ): Analysis {
    if (depth > 8 || word(tokens[0]) !== "SELECT") throw unsupported();
    const from = tokens.findIndex((token) => word(token) === "FROM");
    if (from < 0) throw unsupported();
    const bindings: Scope = [];
    const references: MissingSqlColumn[] = [];
    const unqualifiedOwners = new Map<MissingSqlColumn, Binding[]>();
    const proofs: QueryOutput[] = [];
    const nestedTables: TableRef[] = [];
    const conditions: { tokens: SqlToken[]; scope: Scope }[] = [];
    const joinAt = (index: number) => {
      const modifiers: string[] = [];
      while (joinModifiers.has(word(tokens[index]))) modifiers.push(word(tokens[index++]));
      return word(tokens[index]) === "JOIN" ? { modifiers, next: index + 1 } : null;
    };
    const remember = (binding: Binding) => {
      if (
        relations >= columnInspectionLimits.relations ||
        bindings.some((item) => item.key === binding.key)
      )
        throw unsupported();
      relations++;
      bindings.push(binding);
    };
    let next = from + 1;
    function absorb(nested: Analysis, outputs: QueryOutput[]) {
      nestedTables.push(...nested.tables);
      references.push(...nested.references);
      for (const [ref, owners] of nested.unqualifiedOwners) unqualifiedOwners.set(ref, owners);
      proofs.push(...nested.proofs, ...outputs);
    }
    function derivedOutputs(body: readonly SqlToken[]): {
      outputs?: QueryOutput[];
      starTable?: TableRef;
      columnList?: readonly string[];
    } {
      const arms = unionArms(body);
      if (!arms) {
        const nested = analyze(body, depth + 1, ctes);
        const star = singleStar(body, nested.bindings);
        if (star && "starTable" in star) {
          absorb(nested, star.outputs ?? []);
          return {
            starTable: star.starTable,
            ...(star.outputs?.length ? { outputs: star.outputs } : {}),
            ...(star.columnList ? { columnList: star.columnList } : {}),
          };
        }
        const outputs = star && "outputs" in star ? star.outputs : project(body, nested.bindings);
        absorb(nested, outputs);
        return { outputs };
      }
      // shortcut: a parenthesized arm withholds.
      if (
        arms.length < 2 ||
        arms.length > columnInspectionLimits.relations ||
        arms.some((arm) => arm.length === 0)
      )
        throw unsupported();
      const anchorNested = analyze(arms[0], depth + 1, ctes);
      const anchorArm = armOutput(arms[0], anchorNested.bindings);
      if (anchorArm.kind === "bare") {
        absorb(anchorNested, anchorArm.extras);
        const shapes = [shapeOf(anchorArm)];
        for (const arm of arms.slice(1)) {
          const nested = analyze(arm, depth + 1, ctes);
          const later = armOutput(arm, nested.bindings);
          acceptLater(shapes[0], shapeOf(later));
          absorb(nested, later.kind === "bare" ? later.extras : later.outputs);
          shapes.push(shapeOf(later));
        }
        setWidths.push(shapes);
        return {
          starTable: anchorArm.starTable,
          ...(anchorArm.extras.length ? { outputs: anchorArm.extras } : {}),
          ...(anchorArm.columnList ? { columnList: anchorArm.columnList } : {}),
        };
      }
      absorb(anchorNested, anchorArm.outputs);
      const shapes = [shapeOf(anchorArm)];
      for (const arm of arms.slice(1)) {
        const nested = analyze(arm, depth + 1, ctes);
        const later = armOutput(arm, nested.bindings);
        acceptLater(shapes[0], shapeOf(later));
        absorb(nested, later.kind === "bare" ? later.extras : later.outputs);
        shapes.push(shapeOf(later));
      }
      setWidths.push(shapes);
      return { outputs: anchorArm.outputs };
    }
    function atom(): Binding {
      if (tokens[next]?.children) {
        const bodyToken = tokens[next];
        const body = clean(bodyToken.children ?? []);
        if (!bodyToken.text.endsWith(")") || word(body[0]) !== "SELECT") throw unsupported();
        next++;
        const explicitAlias = word(tokens[next]) === "AS";
        if (explicitAlias) next++;
        const alias = nameToken(tokens[next]) ? tokens[next++] : undefined;
        if (!alias) throw unsupported();
        checkName(alias);
        const produced = derivedOutputs(body);
        const outputOf = identifierValue(readIdentifier(alias.text), context.engine);
        const binding: Binding = produced.starTable
          ? {
              key: key(alias.text),
              label: alias.text,
              table: produced.starTable,
              outputOf,
              starTable: produced.starTable,
              ...(produced.outputs?.length ? { outputs: produced.outputs } : {}),
              ...(produced.columnList ? { columnList: produced.columnList } : {}),
            }
          : {
              key: key(alias.text),
              label: alias.text,
              table: produced.outputs![0].table,
              outputOf,
              outputs: produced.outputs,
            };
        remember(binding);
        return binding;
      }
      const relation: SqlToken[] = [];
      if (!nameToken(tokens[next])) throw unsupported();
      relation.push(tokens[next++]);
      while (tokens[next]?.text === "." && nameToken(tokens[next + 1])) {
        relation.push(tokens[next + 1]);
        next += 2;
      }
      if (relation.length > 2) throw unsupported();
      const explicitAlias = word(tokens[next]) === "AS";
      if (explicitAlias) next++;
      const alias = nameToken(tokens[next]) ? tokens[next++] : undefined;
      if (explicitAlias && !alias) throw unsupported();
      for (const token of [...relation, ...(alias ? [alias] : [])]) checkName(token);
      const prior =
        relation.length === 1 ? ctes.find((item) => item.key === key(relation[0].text)) : undefined;
      if (prior) {
        const shown = alias ?? relation[0];
        const binding: Binding = {
          key: key(shown.text),
          label: shown.text,
          table: prior.table,
          outputOf: alias
            ? identifierValue(readIdentifier(alias.text), context.engine)
            : prior.outputOf,
          ...(prior.starTable
            ? {
                starTable: prior.starTable,
                ...(prior.outputs?.length ? { outputs: prior.outputs } : {}),
                ...(prior.columnList ? { columnList: prior.columnList } : {}),
              }
            : { outputs: prior.outputs }),
        };
        remember(binding);
        return binding;
      }
      // Never substitute a guessed search path for observed visibility.
      if (relation.length === 1 && context.unqualifiedTables === undefined)
        throw new Error("Visible-table metadata is unavailable; no column claim was made.");
      const table = lookup.resolve(relation.map((token) => readIdentifier(token.text)));
      const info = table && context.tables.find((item) => tableKey(item.table) === tableKey(table));
      if (
        !table ||
        !info ||
        !["table", "partitionedTable", "r", "p", "BASE TABLE"].includes(info.kind)
      )
        throw new Error(
          "A unique direct base table could not be resolved; no column claim was made.",
        );
      const shown = alias ?? relation.at(-1)!;
      const binding = {
        key: key(shown.text),
        label: shown.text,
        table,
        ...(relation.length === 2 && !alias ? { writtenSchema: key(relation[0].text) } : {}),
      };
      remember(binding);
      return binding;
    }
    let operand: Scope = [atom()];
    const pending: { left: Scope; optional: boolean }[] = [];
    const finishJoins = () => {
      while (pending.length) {
        const join = pending.pop()!;
        if (!join.optional) throw unsupported();
        operand = [...join.left, ...operand];
      }
    };
    while (next < tokens.length && !clauses.has(word(tokens[next])) && tokens[next].text !== ";") {
      if (tokens[next].text === ",") {
        finishJoins();
        next++;
        operand = [atom()];
        continue;
      }
      const join = joinAt(next);
      if (join) {
        const { modifiers } = join;
        if (
          !joins.has(modifiers.join(" ")) ||
          (context.engine === "mysql" && modifiers.includes("FULL"))
        )
          throw unsupported();
        const natural = modifiers.includes("NATURAL"),
          cross = modifiers.includes("CROSS");
        pending.push({
          left: operand,
          optional:
            context.engine === "mysql" &&
            !modifiers.some((m) => ["LEFT", "RIGHT", "FULL"].includes(m)),
        });
        next = join.next;
        operand = [atom()];
        // CROSS/NATURAL joins close immediately. MySQL permits CROSS ... ON/USING.
        if (
          natural ||
          (cross && !(context.engine === "mysql" && ["ON", "USING"].includes(word(tokens[next]))))
        )
          operand = [...pending.pop()!.left, ...operand];
        continue;
      }
      const condition = word(tokens[next++]);
      if (condition !== "ON" && condition !== "USING") throw unsupported();
      const owner = pending.pop();
      if (!owner) throw unsupported();
      operand = [...owner.left, ...operand];
      if (condition === "USING") {
        const list = tokens[next++];
        const names = list?.children && clean(list.children);
        // The lexer classifies unqualified names such as id as dialect keywords.
        const usingName = (t: SqlToken) =>
          nameToken(t) ||
          (["Keyword", "Type", "Builtin"].includes(t.kind) &&
            /^[A-Za-z_][A-Za-z0-9_$]*$/.test(t.text) &&
            ![
              "SELECT",
              "FROM",
              "WITH",
              "WHERE",
              "JOIN",
              "ON",
              "USING",
              "AS",
              "INSERT",
              "UPDATE",
              "DELETE",
            ].includes(t.text.toUpperCase()));
        if (
          !list?.text.endsWith(")") ||
          !names?.length ||
          names.length % 2 === 0 ||
          names.some((t, i) => (i % 2 ? t.text !== "," : !usingName(t)))
        )
          throw unsupported();
        for (const token of names.filter((_, i) => i % 2 === 0)) checkName(token);
      } else {
        const start = next;
        while (
          next < tokens.length &&
          !clauses.has(word(tokens[next])) &&
          tokens[next].text !== ";" &&
          tokens[next].text !== "," &&
          !["ON", "USING"].includes(word(tokens[next])) &&
          !joinAt(next)
        )
          next++;
        if (next === start) throw unsupported();
        conditions.push({ tokens: tokens.slice(start, next), scope: operand });
      }
    }
    finishJoins();
    const forbidden = new Set([
      "SELECT",
      "WITH",
      "JOIN",
      "UNION",
      "INTERSECT",
      "EXCEPT",
      "INTO",
      "LATERAL",
      "APPLY",
      "WINDOW",
      "QUALIFY",
      "TABLESAMPLE",
    ]);
    // Unquoted clause words and type names are not column references.
    const skipBare = new Set([
      ...forbidden,
      ...clauses,
      "AND",
      "OR",
      "NOT",
      "IS",
      "IN",
      "LIKE",
      "ILIKE",
      "BETWEEN",
      "NULL",
      "TRUE",
      "FALSE",
      "ASC",
      "DESC",
      "NULLS",
      "FIRST",
      "LAST",
      "ON",
      "USING",
      "BY",
      "AS",
      "WHEN",
      "THEN",
      "ELSE",
      "CASE",
      "END",
      "EXISTS",
      "INNER",
      "LEFT",
      "RIGHT",
      "FULL",
      "OUTER",
      "CROSS",
      "NATURAL",
      "TEXT",
      "INT",
      "INTEGER",
      "BIGINT",
      "BOOLEAN",
      "NUMERIC",
      "DATE",
      "TIMESTAMP",
      "VARCHAR",
    ]);
    function noteUnqualified(token: SqlToken, owners: Binding[]) {
      if (context.engine === "mysql" && readIdentifier(token.text).value.toLowerCase() === "_rowid")
        throw unsupported();
      checkName(token);
      if (!owners.length) throw unsupported();
      const ref: MissingSqlColumn = {
        name: identifierValue(readIdentifier(token.text), context.engine),
        qualifier: owners.length === 1 ? owners[0].label : "",
        table: owners[0].table,
        from: token.from,
        to: token.to,
        line: state.doc.lineAt(token.from).number,
        ...(owners.length === 1 ? projectedFields(owners[0]) : {}),
      };
      references.push(ref);
      if (owners.length === 1) rememberStar(ref, owners[0]);
      if (owners.length !== 1) unqualifiedOwners.set(ref, [...owners]);
    }
    function scan(raw: readonly SqlToken[], depth: number, scope: Scope, bare: boolean) {
      if (depth > 16) throw unsupported();
      const list = clean(raw);
      for (let i = 0; i < list.length; i++) {
        if (++visited > 4096) throw unsupported();
        const token = list[i];
        if (forbidden.has(word(token))) throw unsupported();
        if (token.children) {
          if (!token.text.endsWith(")")) throw unsupported();
          // A group or call exposes its names. A nested SELECT still withholds.
          scan(token.children, depth + 1, scope, true);
          continue;
        }
        if (
          bare &&
          nameLike(token) &&
          list[i - 1]?.text !== "." &&
          list[i + 1]?.text !== "." &&
          !list[i + 1]?.children &&
          token.kind !== "Type" &&
          (token.kind === "QuotedIdentifier" || !skipBare.has(word(token)))
        )
          noteUnqualified(token, scope);
        if (!nameToken(token) || list[i + 1]?.text !== ".") continue;
        const column = list[i + 2];
        if (column?.text === "*") {
          i += 2;
          continue;
        }
        // schema.table.* is a select-list star. The same shape elsewhere withholds.
        if (nameToken(column) && list[i + 3]?.text === "." && list[i + 4]?.text === "*") {
          if (bare) throw unsupported();
          i += 4;
          continue;
        }
        if (!nameToken(column) || list[i + 3]?.text === ".") throw unsupported();
        checkName(token);
        checkName(column);
        // Qualified function names are not column references; their arguments
        // are still visited on the next iteration.
        if (!list[i + 3]?.children) {
          if (
            context.engine === "mysql" &&
            readIdentifier(column.text).value.toLowerCase() === "_rowid"
          )
            throw unsupported();
          const binding = scope.find((item) => item.key === key(token.text));
          if (!binding)
            throw new Error(
              "An unresolved qualifier prevents this inspection; no column claim was made.",
            );
          const ref: MissingSqlColumn = {
            name: identifierValue(readIdentifier(column.text), context.engine),
            qualifier: token.text,
            table: binding.table,
            from: column.from,
            to: column.to,
            line: state.doc.lineAt(column.from).number,
            ...projectedFields(binding),
          };
          references.push(ref);
          rememberStar(ref, binding);
        }
        i += 2;
      }
    }
    scan(tokens.slice(1, from), 0, bindings, false);
    for (const condition of conditions) scan(condition.tokens, 0, condition.scope, true);
    scan(tokens.slice(next), 0, bindings, true);
    // Top-level select-list names only. Expressions, stars and other clauses stay out.
    let selectAt = 1;
    if (word(tokens[1]) === "DISTINCT" || word(tokens[1]) === "ALL") selectAt = 2;
    const select = tokens.slice(selectAt, from);
    let itemStart = 0;
    for (let i = 0; i <= select.length; i++) {
      if (i < select.length && select[i].text !== ",") continue;
      const item = select.slice(itemStart, i);
      itemStart = i + 1;
      if (!item.length) throw unsupported();
      let column = item;
      if (item.length >= 3 && word(item.at(-2)) === "AS" && nameLike(item.at(-1)))
        column = item.slice(0, -2);
      else if (item.length >= 2 && nameLike(item.at(-1)) && item.at(-2)?.text !== ".")
        column = item.slice(0, -1);
      if (column.length !== 1 || !nameLike(column[0])) continue;
      noteUnqualified(column[0], bindings);
    }
    return {
      references,
      unqualifiedOwners,
      proofs,
      bindings,
      tables: [
        ...new Map(
          [
            ...bindings.filter((binding) => !binding.outputs).map((binding) => binding.table),
            ...nestedTables,
          ].map((table) => [tableKey(table), table]),
        ).values(),
      ],
    };
  }
  /** Names between the CTE name and AS. A star, gap, or repeated name withholds. */
  function columnNames(token: SqlToken): string[] {
    if (!token.text.endsWith(")")) throw unsupported();
    const children = clean(token.children ?? []);
    const names: string[] = [];
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      if (i % 2 === 0) {
        // CodeMirror tags some names, such as id, as keywords.
        if (!nameLike(child)) throw unsupported();
        checkName(child);
        names.push(identifierValue(readIdentifier(child.text), context.engine));
      } else if (child.text !== ",") throw unsupported();
    }
    if (!names.length || children.length % 2 === 0) throw unsupported();
    if (new Set(names.map(columnKey)).size !== names.length) throw unsupported();
    return names;
  }
  function peel(list: readonly SqlToken[]) {
    if (word(list[0]) !== "WITH") return null;
    let index = 1;
    const recursive = word(list[index]) === "RECURSIVE";
    if (recursive) index++;
    const defs: { name: SqlToken; body: SqlToken[]; columns?: string[] }[] = [];
    for (;;) {
      const name = list[index++];
      if (!nameToken(name)) throw unsupported();
      checkName(name);
      const columns = list[index]?.children ? columnNames(list[index++]) : undefined;
      if (word(list[index++]) !== "AS") throw unsupported();
      while (word(list[index]) === "NOT" || word(list[index]) === "MATERIALIZED") index++;
      const bodyToken = list[index++];
      const body = bodyToken?.children && clean(bodyToken.children);
      if (!body || !bodyToken.text.endsWith(")") || word(body[0]) !== "SELECT") throw unsupported();
      defs.push({ name, body, columns });
      if (list[index]?.text !== ",") break;
      index++;
    }
    return { defs, rest: list.slice(index), recursive };
  }
  // Set operators in this body only. A nested query stays inside one parenthesis token.
  function unionArms(body: readonly SqlToken[]): SqlToken[][] | null {
    const arms: SqlToken[][] = [];
    let start = 0;
    let found = false;
    for (let i = 0; i < body.length; i++) {
      const op = word(body[i]);
      if (op !== "UNION" && op !== "INTERSECT" && op !== "EXCEPT") continue;
      found = true;
      const qualifier = word(body[i + 1]);
      const after = qualifier === "ALL" || qualifier === "DISTINCT" ? i + 2 : i + 1;
      arms.push(body.slice(start, i));
      start = after;
      i = after - 1;
    }
    if (!found) return null;
    arms.push(body.slice(start));
    return arms;
  }
  const leading = peel(tokens);
  const recursiveWith = leading?.recursive ?? false;
  const ctes: Binding[] = [];
  const cteReferences: MissingSqlColumn[] = [];
  const cteOwners = new Map<MissingSqlColumn, Binding[]>();
  const cteProofs: QueryOutput[] = [];
  const cteTables: TableRef[] = [];
  function isSchemaStar(item: readonly SqlToken[]) {
    return (
      item.length === 5 &&
      item[1].text === "." &&
      item[3].text === "." &&
      item[4].text === "*" &&
      nameLike(item[0]) &&
      nameLike(item[2])
    );
  }
  function isStarItem(item: readonly SqlToken[]) {
    return (
      (item.length === 1 && item[0].text === "*") ||
      (item.length === 3 && item[1].text === "." && item[2].text === "*" && nameLike(item[0])) ||
      isSchemaStar(item)
    );
  }
  /** One `*`, `alias.*`, or unaliased `schema.table.*`. Only the first two may sit beside simple columns. */
  function singleStar(
    list: readonly SqlToken[],
    scope: Scope,
  ):
    | { starTable: TableRef; outputs?: QueryOutput[]; columnList?: readonly string[] }
    | {
        outputs: QueryOutput[];
      }
    | null {
    if (word(list[0]) !== "SELECT") return null;
    let at = 1;
    if (word(list[1]) === "DISTINCT" || word(list[1]) === "ALL") at = 2;
    const from = list.findIndex((token) => word(token) === "FROM");
    if (from < at || from - at > 2048) return null;
    const items = selectItems(list.slice(at, from));
    const stars = items.filter(isStarItem);
    if (stars.length === 0) return null;
    if (stars.length > 1) throw unsupported();
    const star = stars[0];
    const schemaStar = isSchemaStar(star);
    let owner: Binding | undefined;
    if (schemaStar) {
      const resolved = lookup.resolve([readIdentifier(star[0].text), readIdentifier(star[2].text)]);
      const found = scope.filter(
        (binding) =>
          !!resolved &&
          binding.writtenSchema === key(star[0].text) &&
          binding.key === key(star[2].text) &&
          tableKey(binding.table) === tableKey(resolved),
      );
      if (found.length === 1) owner = found[0];
    } else if (star.length === 1) owner = scope.length === 1 ? scope[0] : undefined;
    else {
      const found = scope.filter((binding) => binding.key === key(star[0].text));
      if (found.length === 1) owner = found[0];
    }
    if (!owner) return null;
    const extras = projectItems(
      items.filter((item) => item !== star),
      scope,
    );
    // shortcut: schema.table.* beside other items withholds, upgrade when that shape is specified.
    if (schemaStar && extras.length) throw unsupported();
    const carried = owner.outputs?.map((output) => ({ ...output })) ?? [];
    if (owner.starTable || !owner.outputs) {
      const outputs = [...(owner.starTable ? carried : []), ...extras];
      if (outputs.length > 256) throw unsupported();
      if (new Set(outputs.map((output) => columnKey(output.name))).size !== outputs.length)
        throw unsupported();
      const starTable = owner.starTable ?? owner.table;
      return {
        starTable,
        ...(outputs.length ? { outputs } : {}),
        ...(owner.columnList ? { columnList: owner.columnList } : {}),
      };
    }
    const outputs = [...carried, ...extras];
    if (outputs.length > 256) throw unsupported();
    if (new Set(outputs.map((output) => columnKey(output.name))).size !== outputs.length)
      throw unsupported();
    return { outputs };
  }
  function armOutput(
    body: readonly SqlToken[],
    scope: Scope,
  ):
    | { kind: "bare"; starTable: TableRef; extras: QueryOutput[]; columnList?: readonly string[] }
    | { kind: "named"; outputs: QueryOutput[] } {
    const star = singleStar(body, scope);
    if (star && "starTable" in star)
      return {
        kind: "bare",
        starTable: star.starTable,
        extras: star.outputs ?? [],
        ...(star.columnList ? { columnList: star.columnList } : {}),
      };
    return {
      kind: "named",
      outputs: star && "outputs" in star ? star.outputs : project(body, scope),
    };
  }
  function shapeOf(
    arm:
      | { kind: "bare"; starTable: TableRef; extras: QueryOutput[] }
      | { kind: "named"; outputs: QueryOutput[] },
  ): SetArm {
    return arm.kind === "bare"
      ? { kind: "bare", starTable: arm.starTable, extras: arm.extras.length }
      : { kind: "named", count: arm.outputs.length };
  }
  function acceptLater(anchor: SetArm, later: SetArm) {
    if (anchor.kind === "named" && later.kind === "named" && anchor.count !== later.count)
      throw unsupported();
    if (
      anchor.kind === "bare" &&
      later.kind === "bare" &&
      tableKey(anchor.starTable) === tableKey(later.starTable) &&
      anchor.extras !== later.extras
    )
      throw unsupported();
  }
  const recordCte = (analyzed: Analysis, extra: QueryOutput[]) => {
    cteReferences.push(...analyzed.references);
    for (const [ref, owners] of analyzed.unqualifiedOwners) cteOwners.set(ref, owners);
    cteProofs.push(...analyzed.proofs, ...extra);
    cteTables.push(...analyzed.tables);
  };
  const rename = (outputs: QueryOutput[], columns?: readonly string[]) => {
    if (!columns) return outputs;
    if (columns.length !== outputs.length) throw unsupported();
    return outputs.map((output, index) => ({ ...output, name: columns[index] }));
  };
  for (const def of leading?.defs ?? []) {
    if (ctes.some((item) => item.key === key(def.name.text))) throw unsupported();
    const arms = unionArms(def.body);
    let outputs: QueryOutput[] | undefined;
    let starTable: TableRef | undefined;
    let columnList: readonly string[] | undefined;
    if (!arms) {
      const analyzed = analyze(def.body, 0, ctes);
      const star = singleStar(def.body, analyzed.bindings);
      if (star && "starTable" in star) {
        recordCte(analyzed, star.outputs ?? []);
        starTable = star.starTable;
        outputs = star.outputs;
        columnList = def.columns ?? star.columnList;
      } else {
        const projected = rename(
          star && "outputs" in star ? star.outputs : project(def.body, analyzed.bindings),
          def.columns,
        );
        recordCte(analyzed, projected);
        outputs = projected;
      }
    } else {
      // shortcut: a non-flat arm withholds. A non-recursive arm cannot read its own CTE.
      if (
        arms.length < 2 ||
        arms.length > columnInspectionLimits.relations ||
        arms.some((arm) => arm.length === 0)
      )
        throw unsupported();
      const anchorAnalyzed = analyze(arms[0], 0, ctes);
      const anchorArm = armOutput(arms[0], anchorAnalyzed.bindings);
      if (anchorArm.kind === "bare") {
        recordCte(anchorAnalyzed, anchorArm.extras);
        columnList = def.columns ?? anchorArm.columnList;
        const scope = [...ctes];
        if (recursiveWith)
          scope.push({
            key: key(def.name.text),
            label: def.name.text,
            table: anchorArm.starTable,
            outputOf: identifierValue(readIdentifier(def.name.text), context.engine),
            starTable: anchorArm.starTable,
            ...(anchorArm.extras.length ? { outputs: anchorArm.extras } : {}),
            ...(columnList ? { columnList } : {}),
          });
        const shapes = [shapeOf(anchorArm)];
        for (const arm of arms.slice(1)) {
          const nested = analyze(arm, 0, scope);
          const later = armOutput(arm, nested.bindings);
          acceptLater(shapes[0], shapeOf(later));
          recordCte(nested, later.kind === "bare" ? later.extras : later.outputs);
          shapes.push(shapeOf(later));
        }
        setWidths.push(shapes);
        starTable = anchorArm.starTable;
        if (anchorArm.extras.length) outputs = anchorArm.extras;
      } else {
        recordCte(anchorAnalyzed, anchorArm.outputs);
        const anchor = rename(anchorArm.outputs, def.columns);
        const scope = [...ctes];
        if (recursiveWith)
          scope.push({
            key: key(def.name.text),
            label: def.name.text,
            table: anchor[0].table,
            outputOf: identifierValue(readIdentifier(def.name.text), context.engine),
            outputs: anchor,
          });
        const shapes = [shapeOf(anchorArm)];
        for (const arm of arms.slice(1)) {
          const nested = analyze(arm, 0, scope);
          const later = armOutput(arm, nested.bindings);
          acceptLater(shapes[0], shapeOf(later));
          recordCte(nested, later.kind === "bare" ? later.extras : later.outputs);
          shapes.push(shapeOf(later));
        }
        setWidths.push(shapes);
        outputs = anchor;
      }
    }
    ctes.push({
      key: key(def.name.text),
      label: def.name.text,
      table: starTable ?? outputs![0].table,
      outputOf: identifierValue(readIdentifier(def.name.text), context.engine),
      ...(starTable
        ? {
            starTable,
            ...(outputs?.length ? { outputs } : {}),
            ...(columnList ? { columnList } : {}),
          }
        : { outputs }),
    });
  }
  const outer = analyze(leading ? leading.rest : tokens, 0, ctes);
  const references = [...cteReferences, ...outer.references];
  const unqualifiedOwners = new Map([...cteOwners, ...outer.unqualifiedOwners]);
  const proofs = [...cteProofs, ...outer.proofs];
  const tables = [
    ...new Map([...cteTables, ...outer.tables].map((table) => [tableKey(table), table])).values(),
  ];
  if (!references.length)
    throw new Error("No supported qualified columns to inspect in this SELECT.");
  const assertCurrent = (current: EditorState) => {
    if (!current.doc.eq(state.doc))
      throw new Error("SQL changed. Inspect the current statement again.");
  };
  return {
    tables,
    finish(current: EditorState, schemas: TableSchema[]): SqlColumnInspection {
      assertCurrent(current);
      const schemaMap = new Map(schemas.map((schema) => [tableKey(schema.table), schema]));
      if (
        schemas.length !== tables.length ||
        schemaMap.size !== tables.length ||
        tables.some((table) => !schemaMap.has(tableKey(table))) ||
        schemas.some(
          (schema) =>
            !schema.columns.length ||
            schema.columns.length > columnInspectionLimits.columnsPerTable ||
            new Set(schema.columns.map((c) => columnKey(c.name))).size !== schema.columns.length,
        ) ||
        schemas.reduce((n, schema) => n + schema.columns.length, 0) >
          columnInspectionLimits.columns ||
        new TextEncoder().encode(JSON.stringify(schemas)).length > columnInspectionLimits.bytes
      )
        throw new Error("Complete table metadata is unavailable; no column claim was made.");
      const columns = new Map(
        schemas.map((schema) => [
          tableKey(schema.table),
          schema.columns.map((column) => column.name),
        ]),
      );
      for (const group of setWidths) {
        const widths = group.map((arm) => {
          if (arm.kind === "named") return arm.count;
          const names = columns.get(tableKey(arm.starTable));
          if (!names?.length)
            throw new Error("Complete table metadata is unavailable; no column claim was made.");
          return names.length + arm.extras;
        });
        if (new Set(widths).size !== 1) throw unsupported();
      }
      const existing = new Map(
        [...columns].map(([table, names]) => [table, new Set(names.map(columnKey))]),
      );
      // PostgreSQL exposes these physical system columns outside attnum > 0.
      // They are valid references, but are not offered as catalog corrections.
      if (context.engine === "postgres")
        for (const names of existing.values())
          for (const name of ["tableoid", "xmin", "cmin", "xmax", "cmax", "ctid"]) names.add(name);
      for (const ref of starRefs) {
        const names = columns.get(tableKey(ref.table));
        if (!names?.length)
          throw new Error("Complete table metadata is unavailable; no column claim was made.");
        const combined = [...names, ...(starExtras.get(ref) ?? [])];
        if (new Set(combined.map(columnKey)).size !== combined.length) throw unsupported();
        const listed = starRenames.get(ref);
        if (listed && listed.length !== combined.length) throw unsupported();
        ref.projectedColumns = listed ?? combined;
      }
      for (const proof of proofs)
        if (!existing.get(tableKey(proof.table))?.has(columnKey(proof.source)))
          throw new Error(
            "A projected column is not in the complete table metadata; no column claim was made.",
          );
      const issues = references.filter((ref) => {
        const owners = unqualifiedOwners.get(ref);
        if (owners) {
          const matches = owners.filter((owner) => {
            const named = owner.outputs?.some(
              (output) => columnKey(output.name) === columnKey(ref.name),
            );
            if (owner.starTable)
              return owner.columnList
                ? owner.columnList.some((name) => columnKey(name) === columnKey(ref.name))
                : !!named || !!existing.get(tableKey(owner.table))?.has(columnKey(ref.name));
            return owner.outputs
              ? !!named
              : !!existing.get(tableKey(owner.table))?.has(columnKey(ref.name));
          });
          if (matches.length !== 1)
            throw new Error(
              "An unqualified column does not resolve to one relation; no column claim was made.",
            );
          return false;
        }
        return ref.projectedColumns
          ? !ref.projectedColumns.some((name) => columnKey(name) === columnKey(ref.name))
          : !existing.get(tableKey(ref.table))!.has(columnKey(ref.name));
      });
      const columnsFor = (issue: MissingSqlColumn) => {
        if (!issues.includes(issue))
          throw new Error("Choose a column from this inspection's table metadata.");
        if (issue.outputOf) {
          if (!issue.projectedColumns?.length)
            throw new Error("Choose a column from this inspection's table metadata.");
          return issue.projectedColumns;
        }
        return columns.get(tableKey(issue.table))!;
      };
      return {
        tables,
        checked: references.length,
        issues,
        columnsFor,
        preview(current, issue, column) {
          assertCurrent(current);
          if (!columnsFor(issue).includes(column))
            throw new Error("Choose a column from this inspection's table metadata.");
          const quote = context.engine === "mysql" ? "`" : '"';
          return {
            from: issue.from,
            to: issue.to,
            insert: quote + column.split(quote).join(quote + quote) + quote,
          };
        },
      };
    },
  };
}
