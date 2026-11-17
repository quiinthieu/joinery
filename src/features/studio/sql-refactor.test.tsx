import { EditorState } from "@codemirror/state";
import { sql } from "@codemirror/lang-sql";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EngineKind, ObjectDetails, SessionInfo, TableSchema } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { sqlDialect } from "./sql-identifiers";
import { analyzeExtractCte, analyzeInlineCte, prepareExpandWildcard } from "./sql-refactor";
import { SqlRefactorDialog } from "./SqlRefactorDialog";
import { ObjectInspectorView } from "./ObjectInspectorView";

function makeEditorState(
  doc: string,
  engine: EngineKind,
  selection: { anchor: number; head?: number },
): EditorState {
  return EditorState.create({
    doc,
    selection: { anchor: selection.anchor, head: selection.head ?? selection.anchor },
    extensions: [sql({ dialect: sqlDialect(engine) })],
  });
}

describe("SQL AST refactoring and workspace Find Usages", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("extracts derived-table and IN-expression subqueries into WITH CTEs with collision checks and quoting", () => {
    const derivedDoc =
      "SELECT paid.id FROM (SELECT id, total_cents FROM public.orders WHERE total_cents > 0) AS paid;";
    const cursorInDerived = derivedDoc.indexOf("SELECT id");
    const derivedState = makeEditorState(derivedDoc, "postgres", { anchor: cursorInDerived });

    const plan = analyzeExtractCte(derivedState, "postgres", ["orders"]);
    expect(plan.replacementKind).toBe("derivedTable");
    expect(plan.defaultName).toBe("extracted_cte");

    // Rejects collision with existing alias `paid` or catalog table `orders`
    expect(() => plan.preview(derivedState, "paid")).toThrow(/collides/i);
    expect(() => plan.preview(derivedState, "orders")).toThrow(/collides/i);

    const preview = plan.preview(derivedState, "paid_orders");
    expect(preview.previewSql).toBe(
      "WITH paid_orders AS (\n  SELECT id, total_cents FROM public.orders WHERE total_cents > 0\n)\nSELECT paid.id FROM paid_orders AS paid;",
    );

    // Expression subquery (`WHERE customer_id IN (SELECT ...)`) replaces with `(SELECT * FROM cte)`
    const exprDoc =
      "WITH active_orgs AS (SELECT id FROM orgs)\nSELECT * FROM orders WHERE org_id IN (SELECT id FROM active_orgs);";
    const subStart = exprDoc.indexOf("(SELECT id FROM active_orgs)");
    const exprState = makeEditorState(exprDoc, "mysql", {
      anchor: subStart,
      head: subStart + "(SELECT id FROM active_orgs)".length,
    });
    const exprPlan = analyzeExtractCte(exprState, "mysql");
    expect(exprPlan.replacementKind).toBe("expressionSubquery");
    const exprPreview = exprPlan.preview(exprState, "org filter");
    expect(exprPreview.previewSql).toContain("`org filter` AS (\n  SELECT id FROM active_orgs\n)");
    expect(exprPreview.previewSql).toContain("WHERE org_id IN (SELECT * FROM `org filter`)");
  });

  it("inserts extracted nested CTEs before the enclosing CTE to preserve non-recursive declaration order", () => {
    const doc =
      "WITH outer_cte AS (SELECT sub.id FROM (SELECT 1 AS id) sub)\nSELECT * FROM outer_cte;";
    const innerPos = doc.indexOf("SELECT 1 AS id");
    const state = makeEditorState(doc, "snowflake", { anchor: innerPos });
    const plan = analyzeExtractCte(state, "snowflake");
    const preview = plan.preview(state, "INNER_SEED");
    expect(preview.previewSql.indexOf("INNER_SEED AS")).toBeLessThan(
      preview.previewSql.indexOf("outer_cte AS"),
    );
  });

  it("rejects correlated outer aliases and recursive CTE self-references while supporting comma-joined derived tables", () => {
    // Correlated outer alias `c.id` cannot be hoisted into a top-level CTE
    const correlatedDoc =
      "SELECT c.id FROM public.customers c WHERE EXISTS (SELECT 1 FROM public.orders o WHERE o.customer_id = c.id);";
    const corrState = makeEditorState(correlatedDoc, "postgres", {
      anchor: correlatedDoc.indexOf("SELECT 1"),
    });
    expect(() => analyzeExtractCte(corrState, "postgres")).toThrow(/correlated subquery/i);

    // Shadowed inner alias `c` IS local to the subquery and extracts cleanly
    const shadowedDoc =
      "SELECT c.id FROM public.customers c WHERE c.id IN (SELECT c.customer_id FROM public.orders c);";
    const shadState = makeEditorState(shadowedDoc, "postgres", {
      anchor: shadowedDoc.indexOf("SELECT c.customer_id"),
    });
    expect(analyzeExtractCte(shadState, "postgres").replacementKind).toBe("expressionSubquery");

    // Subquery inside a WITH RECURSIVE CTE referencing that recursive CTE itself cannot precede it
    const recDoc =
      "WITH RECURSIVE tree AS (SELECT 1 AS n UNION ALL SELECT sub.n FROM (SELECT n + 1 AS n FROM tree WHERE n < 5) sub) SELECT * FROM tree;";
    const recState = makeEditorState(recDoc, "postgres", {
      anchor: recDoc.indexOf("SELECT n + 1"),
    });
    expect(() => analyzeExtractCte(recState, "postgres")).toThrow(
      /recursive subquery that references enclosing CTE/i,
    );

    // Comma-joined derived table (`FROM a, (SELECT ...) b`) is recognized as a derivedTable, not expressionSubquery
    const commaDoc =
      "SELECT a.id, b.cnt FROM public.customers a, (SELECT COUNT(*) AS cnt FROM public.orders) b;";
    const commaState = makeEditorState(commaDoc, "postgres", {
      anchor: commaDoc.indexOf("SELECT COUNT"),
    });
    const commaPlan = analyzeExtractCte(commaState, "postgres");
    expect(commaPlan.replacementKind).toBe("derivedTable");
    expect(commaPlan.preview(commaState, "order_counts").previewSql).toBe(
      "WITH order_counts AS (\n  SELECT COUNT(*) AS cnt FROM public.orders\n)\nSELECT a.id, b.cnt FROM public.customers a, order_counts b;",
    );
  });

  it("expands *, alias.*, and local CTE wildcards with dialect-accurate identifier quoting", () => {
    const ordersSchema: TableSchema = {
      table: { schema: "public", name: "orders" },
      columns: [
        {
          id: "order_id",
          name: "order_id",
          databaseType: "bigint",
          kind: "integer",
          nullable: false,
          primaryKey: true,
          foreignKey: false,
          editable: true,
          insertable: true,
          hasDefault: true,
        },
        {
          id: "Order Total",
          name: "Order Total",
          databaseType: "numeric",
          kind: "decimal",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: true,
          insertable: true,
          hasDefault: false,
        },
      ],
      primaryKey: ["id"],
      foreignKeys: [],
      editable: true,
    };
    const customersSchema: TableSchema = {
      table: { schema: "public", name: "customers" },
      columns: [
        {
          id: "order_id",
          name: "order_id",
          databaseType: "bigint",
          kind: "integer",
          nullable: false,
          primaryKey: true,
          foreignKey: false,
          editable: true,
          insertable: true,
          hasDefault: true,
        },
      ],
      primaryKey: ["id"],
      foreignKeys: [],
      editable: true,
    };
    const tables = [
      {
        table: { schema: "public", name: "orders" },
        kind: "table",
        estimatedRows: 10,
        totalBytes: null,
      },
      {
        table: { schema: "public", name: "customers" },
        kind: "table",
        estimatedRows: 5,
        totalBytes: null,
      },
    ];

    // Single unaliased table -> unqualified column names
    const singleSql = "SELECT * FROM public.orders WHERE id > 0;";
    const singleState = makeEditorState(singleSql, "postgres", {
      anchor: singleSql.indexOf("*"),
    });
    const singlePlan = prepareExpandWildcard(singleState, singleSql.indexOf("*"), {
      engine: "postgres",
      tables,
      defaultSchema: "public",
    });
    expect(singlePlan.tablesToFetch).toEqual([{ schema: "public", name: "orders" }]);
    const singleDone = singlePlan.finish(singleState, [ordersSchema]);
    expect(singleDone.insert).toBe('order_id, "Order Total"');
    expect(singleDone.previewSql).toBe(
      'SELECT order_id, "Order Total" FROM public.orders WHERE id > 0;',
    );

    // Qualified `o.*` in a JOIN -> qualified with `o.`
    const joinSql =
      "SELECT o.*, c.id AS customer_pk FROM public.orders o JOIN public.customers c ON c.id = o.id;";
    const joinState = makeEditorState(joinSql, "postgres", {
      anchor: joinSql.indexOf("o.*") + 2,
    });
    const joinPlan = prepareExpandWildcard(joinState, joinSql.indexOf("o.*") + 2, {
      engine: "postgres",
      tables,
      defaultSchema: "public",
    });
    expect(joinPlan.tablesToFetch).toEqual([{ schema: "public", name: "orders" }]);
    const joinDone = joinPlan.finish(joinState, [ordersSchema, customersSchema]);
    expect(joinDone.insert).toBe('o.order_id, o."Order Total"');

    // Local CTE wildcard expansion requires zero remote table schemas
    const cteSql =
      "WITH summary AS (SELECT o.id AS order_id, o.total AS amount FROM raw_t o) SELECT * FROM summary;";
    const starPos = cteSql.lastIndexOf("*");
    const cteState = makeEditorState(cteSql, "postgres", { anchor: starPos });
    const ctePlan = prepareExpandWildcard(cteState, starPos, {
      engine: "postgres",
      tables: [],
      defaultSchema: "public",
    });
    expect(ctePlan.tablesToFetch).toEqual([]);
    const cteDone = ctePlan.finish(cteState, []);
    expect(cteDone.insert).toBe("order_id, amount");

    // Self-join (`FROM public.orders o1 JOIN public.orders orders`) expands `orders.*` once (only the `orders` alias)
    const selfJoinSql =
      "SELECT orders.* FROM public.orders o1 JOIN public.orders orders ON orders.id = o1.id;";
    const selfStarPos = selfJoinSql.indexOf("orders.*") + "orders.".length;
    const selfState = makeEditorState(selfJoinSql, "postgres", { anchor: selfStarPos });
    const selfPlan = prepareExpandWildcard(selfState, selfStarPos, {
      engine: "postgres",
      tables,
      defaultSchema: "public",
    });
    expect(selfPlan.finish(selfState, [ordersSchema]).insert).toBe(
      'orders.order_id, orders."Order Total"',
    );

    // Table hidden behind explicit alias (`FROM public.orders o1`) rejects `orders.*`
    const hiddenSql = "SELECT orders.* FROM public.orders o1;";
    const hiddenPos = hiddenSql.indexOf("orders.*") + "orders.".length;
    const hiddenState = makeEditorState(hiddenSql, "postgres", { anchor: hiddenPos });
    expect(() =>
      prepareExpandWildcard(hiddenState, hiddenPos, {
        engine: "postgres",
        tables,
        defaultSchema: "public",
      }),
    ).toThrow(/Qualifier "orders" does not match any table or alias/i);
  });

  it("renders SqlRefactorDialog and ObjectInspectorView workspace SQL usages navigation", async () => {
    const doc = "SELECT sub.id FROM (SELECT 1 AS id) sub;";
    const state = makeEditorState(doc, "postgres", { anchor: doc.indexOf("SELECT 1") });
    const plan = analyzeExtractCte(state, "postgres");
    const onApply = vi.fn();
    render(
      <SqlRefactorDialog
        mode={{ kind: "extractCte", plan, state }}
        onApplyChanges={onApply}
        onClose={() => {}}
      />,
    );
    const input = screen.getByLabelText("CTE name");
    fireEvent.change(input, { target: { value: "seed_ids" } });
    expect(screen.getByLabelText("Extract CTE preview").textContent).toContain("WITH seed_ids AS");
    fireEvent.click(screen.getByRole("button", { name: "Extract CTE" }));
    expect(onApply).toHaveBeenCalledTimes(1);
    cleanup();

    // ObjectInspectorView Find SQL usages
    const openSavedQuerySpy = vi.fn().mockReturnValue("saved-tab-1");
    useStudio.setState({
      tabs: [
        {
          id: "q-tab-1",
          title: "Order Revenue",
          kind: "query",
          connectionId: "conn-1",
          sql: "SELECT * FROM public.orders;",
          table: null,
          object: null,
          schema: null,
          view: "data",
          results: [],
          activeResultId: null,
          pages: [],
          pageKeys: {},
          query: { filters: [], sort: [], whereClause: null, orderClause: null },
          loadedRows: 0,
          nextKey: null,
          hasMore: false,
          loadingPage: false,
          dataSessionId: null,
          querySession: null,
          querySessionClosed: false,
          fileToken: null,
          fileName: null,
          savedSql: null,
          savedQueryId: null,
          runId: null,
          status: "idle",
          error: null,
          metrics: {},
          elapsedMs: 0,
          revision: 0,
        },
      ],
      activeTabId: "obj-tab",
      openSavedQuery: openSavedQuerySpy,
    });

    vi.spyOn(client, "call").mockImplementation(async (cmd) => {
      if (cmd === "find_sql_usages") {
        return {
          scannedDocuments: 2,
          skippedStatements: 0,
          truncated: false,
          hits: [
            {
              documentId: "q-tab-1",
              documentTitle: "Order Revenue",
              sourceKind: "openTab",
              savedQueryId: null,
              role: "SELECT",
              span: { startUtf16: 21, endUtf16: 27, line: 1, column: 22 },
              lineText: "SELECT * FROM public.orders;",
            },
            {
              documentId: "sq-9",
              documentTitle: "Monthly Audit",
              sourceKind: "savedQuery",
              savedQueryId: "sq-9",
              role: "UPDATE",
              span: { startUtf16: 7, endUtf16: 13, line: 1, column: 8 },
              lineText: "UPDATE orders SET audited = true;",
            },
          ],
        } as never;
      }
      throw new Error(`Unexpected IPC: ${cmd}`);
    });

    const objTab: StudioTab = {
      id: "obj-tab",
      title: "orders",
      kind: "object",
      connectionId: "conn-1",
      sql: "",
      table: null,
      object: { schema: "public", name: "orders", kind: "table", identity: null },
      schema: null,
      view: "structure",
      results: [],
      activeResultId: null,
      pages: [],
      pageKeys: {},
      query: { filters: [], sort: [], whereClause: null, orderClause: null },
      loadedRows: 0,
      nextKey: null,
      hasMore: false,
      loadingPage: false,
      dataSessionId: null,
      querySession: null,
      querySessionClosed: false,
      fileToken: null,
      fileName: null,
      savedSql: null,
      savedQueryId: null,
      runId: null,
      status: "idle",
      error: null,
      metrics: {},
      elapsedMs: 0,
      revision: 0,
    };
    const pgSession: SessionInfo = {
      id: "sess-1",
      connectionId: "conn-1",
      engine: "postgres",
      serverVersion: "PostgreSQL 18",
      latencyMs: 1,
      encrypted: false,
      capabilities: {
        sql: true,
        transactions: true,
        cancellation: true,
        tableBrowsing: true,
        tableEditing: true,
        tableFiltering: true,
        processes: true,
        objectExplorer: true,
      },
    };
    const details: ObjectDetails = {
      object: { schema: "public", name: "orders", kind: "table", identity: null },
      ownerTable: null,
      comment: null,
      ddl: { status: "available", ddl: "CREATE TABLE public.orders (id int);", truncated: false },
      constraints: [],
      indexes: [],
      triggers: [],
      routine: null,
      sequence: null,
      uses: [],
      usedBy: [],
    };

    render(<ObjectInspectorView tab={objTab} session={pgSession} providedDetails={details} />);
    fireEvent.click(screen.getByRole("button", { name: /Find SQL usages/i }));

    await waitFor(() => {
      expect(screen.getByText("Order Revenue · Ln 1")).toBeTruthy();
    });
    expect(screen.getByText("Monthly Audit · Ln 1")).toBeTruthy();

    fireEvent.click(screen.getByText("Order Revenue · Ln 1"));
    expect(useStudio.getState().activeTabId).toBe("q-tab-1");

    fireEvent.click(screen.getByText("Monthly Audit · Ln 1"));
    expect(openSavedQuerySpy).toHaveBeenCalledWith("sq-9");
  });

  it("inlines single and multi-CTE definitions preserving explicit aliases, adding implicit aliases, and rejecting recursive/materialized CTEs", () => {
    // 1. Single CTE without explicit alias at call site -> removes WITH and adds `AS active_orders` so `active_orders.id` keeps resolving!
    const singleDoc =
      "WITH active_orders AS (SELECT id, total_cents FROM public.orders WHERE status = 'open')\nSELECT active_orders.id FROM active_orders WHERE active_orders.total_cents > 100;";
    const stateOnHeader = makeEditorState(singleDoc, "postgres", {
      anchor: singleDoc.indexOf("active_orders"),
    });
    const planSingle = analyzeInlineCte(stateOnHeader, "postgres");
    expect(planSingle.cteName).toBe("active_orders");
    expect(planSingle.referenceCount).toBe(1);
    expect(planSingle.previewSql).toBe(
      "SELECT active_orders.id FROM (SELECT id, total_cents FROM public.orders WHERE status = 'open') AS active_orders WHERE active_orders.total_cents > 100;",
    );

    // 2. Multi-CTE: cursor on FROM reference inlines only that CTE and preserves explicit alias `ao` + remaining CTE `vip_users`
    const multiDoc =
      "WITH active_orders AS (SELECT id, customer_id FROM orders), vip_users AS (SELECT id FROM customers WHERE vip = 1)\nSELECT ao.id FROM active_orders AS ao JOIN vip_users v ON v.id = ao.customer_id;";
    const stateOnRef = makeEditorState(multiDoc, "sqlite", {
      anchor: multiDoc.indexOf("active_orders AS ao") + 2,
    });
    const planMulti = analyzeInlineCte(stateOnRef, "sqlite");
    expect(planMulti.cteName).toBe("active_orders");
    expect(planMulti.previewSql).toBe(
      "WITH vip_users AS (SELECT id FROM customers WHERE vip = 1)\nSELECT ao.id FROM (SELECT id, customer_id FROM orders) AS ao JOIN vip_users v ON v.id = ao.customer_id;",
    );

    // 3. Rejects recursive CTEs and explicit column-list CTEs
    const recDoc =
      "WITH RECURSIVE nums AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM nums WHERE n < 5) SELECT * FROM nums;";
    expect(() =>
      analyzeInlineCte(
        makeEditorState(recDoc, "postgres", { anchor: recDoc.indexOf("nums") }),
        "postgres",
      ),
    ).toThrow(/recursive/i);

    const colListDoc = "WITH renamed(a, b) AS (SELECT 1, 2) SELECT a, b FROM renamed;";
    expect(() =>
      analyzeInlineCte(
        makeEditorState(colListDoc, "postgres", { anchor: colListDoc.indexOf("renamed") }),
        "postgres",
      ),
    ).toThrow(/column rename list/i);

    const matDoc =
      "WITH fenced AS MATERIALIZED (SELECT id FROM public.orders) SELECT * FROM fenced;";
    expect(() =>
      analyzeInlineCte(
        makeEditorState(matDoc, "postgres", { anchor: matDoc.indexOf("fenced") }),
        "postgres",
      ),
    ).toThrow(/materialized/i);

    // 4. SqlRefactorDialog renders Inline CTE preview and dispatches `input.inline-cte`
    const onApply = vi.fn();
    render(
      <SqlRefactorDialog
        mode={{ kind: "inlineCte", plan: planSingle }}
        onApplyChanges={onApply}
        onClose={() => {}}
      />,
    );
    expect(screen.getByLabelText("Inline CTE preview").textContent).toContain("AS active_orders");
    fireEvent.click(screen.getByRole("button", { name: "Inline CTE" }));
    expect(onApply).toHaveBeenCalledWith(planSingle.changes, "input.inline-cte");
  });
});
