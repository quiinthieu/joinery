import { test, expect, type Page } from "@playwright/test";

const BRIDGE_URL = "http://127.0.0.1:1422";
const BRIDGE_TOKEN = "joinery-disposable-browser-tests-only";

async function installClipboardMock(page: Page) {
  await page.addInitScript(() => {
    let text = "";
    const w = window as unknown as {
      __joineryCopiedText: () => string;
      __joineryClearCopiedText: () => void;
    };
    w.__joineryCopiedText = () => text;
    w.__joineryClearCopiedText = () => {
      text = "";
    };
    const mockClipboard = {
      writeText: async (value: string) => {
        text = value;
      },
      readText: async () => text,
    };
    try {
      Object.defineProperty(Navigator.prototype, "clipboard", {
        configurable: true,
        get: () => mockClipboard,
      });
    } catch {
      // Fallback if prototype property cannot be redefined
    }
    try {
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        get: () => mockClipboard,
      });
    } catch {
      // Prototype getter handles access
    }
  });
}

async function connectPostgres(page: Page, name: string, storeQueryText = false) {
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Database engine").selectOption("postgres");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill("55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  if (storeQueryText) {
    await page.getByLabel(/Save SQL text in local history/).check();
  }
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
}

async function connectMysql(page: Page, name: string, port = 53306) {
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Database engine").selectOption("mysql");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(port));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
}

test("PostgreSQL explorer categorizes objects, navigates by keyboard, and inspects structure, DDL and dependencies", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await installClipboardMock(page);
  await page.goto("/");
  await connectPostgres(page, "Explorer PG");

  const cleanupSql = `
    DROP MATERIALIZED VIEW IF EXISTS public.e2e_r3_summary_mv CASCADE;
    DROP VIEW IF EXISTS public.e2e_r3_active_v CASCADE;
    DROP TABLE IF EXISTS public.e2e_r3_items CASCADE;
    DROP SEQUENCE IF EXISTS public.e2e_r3_seq CASCADE;
    DROP FUNCTION IF EXISTS public.e2e_r3_touch_fn() CASCADE;
    DROP FUNCTION IF EXISTS public.e2e_r3_calc(integer) CASCADE;
    DROP FUNCTION IF EXISTS public.e2e_r3_calc(numeric, integer) CASCADE;
  `;

  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(`
    ${cleanupSql}
    CREATE SEQUENCE public.e2e_r3_seq AS bigint START WITH 10 INCREMENT BY 5;
    CREATE TABLE public.e2e_r3_items (
      id BIGINT PRIMARY KEY DEFAULT nextval('public.e2e_r3_seq'),
      note TEXT NOT NULL,
      amount INTEGER NOT NULL CONSTRAINT e2e_r3_amount_positive CHECK (amount >= 0)
    );
    CREATE INDEX e2e_r3_lower_note_idx
      ON public.e2e_r3_items (lower(note))
      WHERE amount > 0;
    CREATE FUNCTION public.e2e_r3_touch_fn() RETURNS trigger
      LANGUAGE plpgsql AS $$
      BEGIN
        RETURN NEW;
      END;
      $$;
    CREATE TRIGGER e2e_r3_touch_trg
      AFTER INSERT OR UPDATE ON public.e2e_r3_items
      FOR EACH ROW EXECUTE FUNCTION public.e2e_r3_touch_fn();
    CREATE FUNCTION public.e2e_r3_calc(x integer) RETURNS integer
      LANGUAGE sql IMMUTABLE AS $$ SELECT x * 2; $$;
    CREATE FUNCTION public.e2e_r3_calc(x numeric, scale integer) RETURNS numeric
      LANGUAGE sql IMMUTABLE AS $$ SELECT round(x, scale); $$;
    CREATE VIEW public.e2e_r3_active_v AS
      SELECT id, note, public.e2e_r3_calc(amount) AS doubled
      FROM public.e2e_r3_items
      WHERE amount > 0;
    CREATE MATERIALIZED VIEW public.e2e_r3_summary_mv AS
      SELECT count(*) AS total_active FROM public.e2e_r3_active_v;
  `);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("affected");

  try {
    await page.getByRole("button", { name: "Refresh schema", exact: true }).click();

    const publicHeading = page.locator('[data-tree-node="schema:public"]');
    await expect(publicHeading).toBeVisible();
    await publicHeading.click();
    await expect(publicHeading).toHaveAttribute("aria-expanded", "true");

    // Filter chips: Routines -> shows overloaded signatures and hides base tables
    const filterGroup = page.getByRole("group", { name: "Object kind filter" });
    await filterGroup.getByRole("button", { name: "Routines", exact: true }).click();
    await expect(
      page
        .locator(".schema-tree")
        .getByRole("button", { name: "e2e_r3_calc(x integer)", exact: true }),
    ).toBeVisible();
    await expect(
      page
        .locator(".schema-tree")
        .getByRole("button", { name: "e2e_r3_calc(x numeric, scale integer)", exact: true }),
    ).toBeVisible();
    await expect(
      page.locator(".schema-tree").getByRole("button", { name: "e2e_r3_items", exact: true }),
    ).toHaveCount(0);

    // Filter chips: Other -> Sequences, Triggers, Indexes
    await filterGroup.getByRole("button", { name: "Other", exact: true }).click();
    await expect(
      page.locator(".schema-tree").getByRole("button", { name: "e2e_r3_seq", exact: true }),
    ).toBeVisible();
    await expect(
      page.locator(".schema-tree").getByRole("button", { name: "e2e_r3_touch_trg", exact: true }),
    ).toBeVisible();
    await expect(
      page
        .locator(".schema-tree")
        .getByRole("button", { name: "e2e_r3_lower_note_idx", exact: true }),
    ).toBeVisible();

    // Reset filter to All and exercise keyboard tree navigation
    await filterGroup.getByRole("button", { name: "All", exact: true }).click();
    await publicHeading.focus();
    await publicHeading.press("ArrowLeft");
    await expect(publicHeading).toHaveAttribute("aria-expanded", "false");
    await publicHeading.press("ArrowRight");
    await expect(publicHeading).toHaveAttribute("aria-expanded", "true");
    await publicHeading.press("ArrowRight");
    const tablesCat = page.locator('[data-tree-node="cat:public:tables"]');
    await expect(tablesCat).toBeFocused();
    await tablesCat.press("ArrowLeft");
    await expect(tablesCat).toHaveAttribute("aria-expanded", "false");
    await tablesCat.press("ArrowRight");
    await expect(tablesCat).toHaveAttribute("aria-expanded", "true");

    // Substring filter + Enter on overloaded function e2e_r3_calc(x integer)
    const searchInput = page.getByLabel("Search tables", { exact: true });
    await searchInput.fill("e2e_r3_calc");
    const intCalcBtn = page
      .locator(".schema-tree")
      .getByRole("button", { name: "e2e_r3_calc(x integer)", exact: true });
    await intCalcBtn.focus();
    await intCalcBtn.press("Enter");

    await expect(
      page.getByRole("heading", { name: "public.e2e_r3_calc(x integer)" }),
    ).toBeVisible();
    await expect(page.getByLabel("Routine properties")).toContainText("IMMUTABLE");

    // Dependency navigation: e2e_r3_calc(integer) -> Used by e2e_r3_active_v -> Uses e2e_r3_items
    const deps = page.getByLabel("Object dependencies");
    await deps.getByRole("button", { name: /public\.e2e_r3_active_v/ }).click();
    await expect(page.locator(".query-context")).toContainText("public.e2e_r3_active_v");
    const viewDeps = page.getByLabel("Object dependencies");
    await expect(viewDeps).toContainText("public.e2e_r3_summary_mv");
    await viewDeps.getByRole("button", { name: /public\.e2e_r3_items/ }).click();

    // Inspect table Structure: Constraints, expression + partial Index, Triggers
    await expect(page.locator(".query-context")).toContainText("public.e2e_r3_items");
    await expect(page.getByRole("table", { name: "Constraints" })).toContainText(
      "e2e_r3_amount_positive",
    );
    const indexesTable = page.getByRole("table", { name: "Indexes" });
    await expect(indexesTable).toContainText("e2e_r3_lower_note_idx");
    await expect(indexesTable).toContainText("lower(note)");
    await expect(indexesTable).toContainText("WHERE amount > 0");
    const triggersTable = page.getByRole("table", { name: "Triggers" });
    await expect(triggersTable).toContainText("e2e_r3_touch_trg");
    await expect(triggersTable).toContainText("FOR EACH ROW");

    // Click trigger link in Structure view -> opens Trigger inspector tab
    await triggersTable.getByRole("button", { name: "e2e_r3_touch_trg", exact: true }).click();
    await expect(page.getByRole("heading", { name: "public.e2e_r3_touch_trg" })).toBeVisible();
    await expect(page.getByLabel("Trigger properties")).toContainText("INSERT OR UPDATE");

    // Return to e2e_r3_items tab and inspect DDL + Copy DDL + Open in SQL editor
    await page
      .getByRole("tablist", { name: "Workspaces" })
      .getByRole("tab", { name: "e2e_r3_items", exact: true })
      .click();
    await page.getByRole("button", { name: "DDL", exact: true }).click();
    const ddlPre = page.getByLabel("Object DDL");
    await expect(ddlPre).toContainText('CREATE TABLE "public"."e2e_r3_items"');
    await expect(ddlPre).toContainText("e2e_r3_lower_note_idx");

    await page.getByRole("button", { name: "Copy DDL", exact: true }).click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as unknown as { __joineryCopiedText: () => string }).__joineryCopiedText(),
        ),
      )
      .toContain('CREATE TABLE "public"."e2e_r3_items"');

    await page.getByRole("button", { name: "Open in SQL editor", exact: true }).click();
    await expect(page.getByRole("textbox", { name: "SQL editor" })).toContainText(
      'CREATE TABLE "public"."e2e_r3_items"',
    );
  } finally {
    await searchInputCleanup(page, cleanupSql);
  }
  expect(errors).toEqual([]);
});

async function searchInputCleanup(page: Page, cleanupSql: string) {
  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(cleanupSql);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("affected");
}

test("Search Everywhere jumps across simultaneous PostgreSQL and MySQL connections, open tabs and query history", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));

  const offlineId = crypto.randomUUID();
  const headers = { Authorization: `Bearer ${BRIDGE_TOKEN}` };
  await request.post(`${BRIDGE_URL}/rpc/save_profile`, {
    headers,
    data: {
      profile: {
        id: offlineId,
        name: "Offline Warehouse R3",
        groupId: "local",
        engine: "postgres",
        host: "127.0.0.1",
        port: 55432,
        database: "joinery",
        username: "joinery",
        environment: "local",
        safeMode: "readOnly",
        tlsMode: "disable",
        caCertPath: null,
        ssh: null,
        rememberPassword: false,
        storeQueryText: false,
        storeDrafts: false,
        snowflake: null,
      },
      passphrase: null,
    },
  });

  try {
    await page.goto("/");
    await connectPostgres(page, "PG Search R3", true);
    await connectMysql(page, "MySQL Search R3", 53306);

    // Record a distinctive query in local history on PG Search R3
    await page.getByRole("button", { name: "New query tab" }).click();
    await page.getByLabel("Query connection").selectOption({ label: "PG Search R3" });
    const markerSql = `SELECT 'r3_everywhere_${Date.now()}'::text AS marker;`;
    await page.getByRole("textbox", { name: "SQL editor" }).fill(markerSql);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");

    // Open Search Everywhere directly in Objects scope via Ctrl+Shift+O
    await page.keyboard.press("Control+Shift+O");
    const palette = page.getByRole("dialog", { name: "Command palette" });
    await expect(palette).toBeVisible();
    await expect(
      palette.getByRole("tablist", { name: "Search scope" }).getByRole("tab", { name: "Objects" }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(palette.locator(".palette-offline-note")).toContainText("connected");
    await expect(palette.locator(".palette-offline-note")).toContainText("offline");

    // Jump to MySQL table "sample"
    const search = palette.getByRole("textbox", { name: "Search commands" });
    await search.fill("MySQL Search R3 sample");
    await search.press("Enter");
    await expect(palette).toHaveCount(0);
    await expect(page.locator(".query-context")).toContainText("joinery.sample");
    await expect(page.locator(".result-footer")).toContainText("500 rows loaded");

    // Inspect MySQL Structure & DDL on joinery.sample
    await page.getByRole("button", { name: "Structure", exact: true }).click();
    await expect(page.getByRole("table", { name: "Indexes" })).toContainText("PRIMARY");
    await page.getByRole("button", { name: "DDL", exact: true }).click();
    await expect(page.getByLabel("Object DDL")).toContainText("CREATE TABLE `sample`");

    // Jump across connections to PostgreSQL table "public.orders"
    await page.keyboard.press("Control+Shift+O");
    await expect(palette).toBeVisible();
    await palette
      .getByRole("textbox", { name: "Search commands" })
      .fill("PG Search R3 public.orders");
    await page.keyboard.press("Enter");
    await expect(palette).toHaveCount(0);
    await expect(page.locator(".query-context")).toContainText("public.orders");

    // Search query history via Ctrl+K -> History scope
    await page.keyboard.press("Control+k");
    await expect(palette).toBeVisible();
    await palette
      .getByRole("tablist", { name: "Search scope" })
      .getByRole("tab", { name: "History" })
      .click();
    await palette.getByRole("textbox", { name: "Search commands" }).fill("r3_everywhere_");
    await page.keyboard.press("Enter");
    await expect(palette).toHaveCount(0);
    await expect(page.getByRole("textbox", { name: "SQL editor" })).toContainText("r3_everywhere_");

    // Switch back to open tab "sample" via Ctrl+K -> Actions scope
    await page.keyboard.press("Control+k");
    await expect(palette).toBeVisible();
    await palette
      .getByRole("tablist", { name: "Search scope" })
      .getByRole("tab", { name: "Actions" })
      .click();
    await palette.getByRole("textbox", { name: "Search commands" }).fill("Switch to tab: sample");
    await page.keyboard.press("Enter");
    await expect(palette).toHaveCount(0);
    await expect(page.locator(".query-context")).toContainText("joinery.sample");
  } finally {
    await request.post(`${BRIDGE_URL}/rpc/delete_profile`, {
      headers,
      data: { id: offlineId, passphrase: null },
    });
  }
  expect(errors).toEqual([]);
});
