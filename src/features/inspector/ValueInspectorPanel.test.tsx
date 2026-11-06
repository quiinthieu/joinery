import { describe, expect, it, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import {
  ValueInspectorPanel,
  detectDefaultMode,
  detectCompatibleModes,
} from "./ValueInspectorPanel";
import type { CellTarget } from "../grid/DatabaseGrid";

afterEach(cleanup);

describe("ValueInspectorPanel", () => {
  const baseTarget: CellTarget = {
    resultId: "res-1",
    row: 0,
    column: {
      id: "c1",
      name: "payload_json",
      databaseType: "variant",
      kind: "json",
      nullable: true,
      primaryKey: false,
      foreignKey: false,
      editable: false,
      insertable: false,
      hasDefault: false,
    },
    value: '{"store":{"book":[{"title":"Snowflake"}]}}',
    large: false,
  };

  it("detects default mode correctly for varied data types", () => {
    // JSON / Variant
    expect(detectDefaultMode(baseTarget)).toBe("json");

    // Binary
    expect(
      detectDefaultMode({
        ...baseTarget,
        column: { ...baseTarget.column, kind: "binary", databaseType: "binary" },
        value: "\\x89PNG\\r\\n",
      }),
    ).toBe("binary");

    // DateTime
    expect(
      detectDefaultMode({
        ...baseTarget,
        column: { ...baseTarget.column, kind: "temporal", databaseType: "timestamp_tz" },
        value: "2026-11-05T15:30:00Z",
      }),
    ).toBe("datetime");

    // Array
    expect(
      detectDefaultMode({
        ...baseTarget,
        column: { ...baseTarget.column, kind: "json", databaseType: "array" },
        value: '["a", "b"]',
      }),
    ).toBe("array");

    // Spatial
    expect(
      detectDefaultMode({
        ...baseTarget,
        column: { ...baseTarget.column, kind: "text", databaseType: "geography" },
        value: "POLYGON ((30 10, 40 40, 20 40, 10 20, 30 10))",
      }),
    ).toBe("spatial");
  });

  it("detects compatible modes list", () => {
    const modes = detectCompatibleModes(baseTarget);
    expect(modes).toContain("raw");
    expect(modes).toContain("json");
  });

  it("renders empty state when target is null", () => {
    render(<ValueInspectorPanel target={null} onClose={vi.fn()} />);
    expect(screen.getByLabelText("Selected cell inspector")).toBeDefined();
    expect(screen.getByText("Select a cell to see its value and type.")).toBeDefined();
  });

  it("renders metadata, column name in h3, and value in .inspector-value", () => {
    render(<ValueInspectorPanel target={baseTarget} onClose={vi.fn()} />);
    const inspector = screen.getByLabelText("Selected cell inspector");
    expect(inspector.className).toContain("inspector");
    expect(screen.getByRole("heading", { level: 3 }).textContent).toBe("payload_json");
    expect(screen.getByText("variant")).toBeDefined();
    expect(inspector.querySelector(".inspector-value")).toBeDefined();
  });

  it("switches inspector mode on tab click", () => {
    render(<ValueInspectorPanel target={baseTarget} onClose={vi.fn()} />);
    const rawTab = screen.getByRole("tab", { name: "RAW" });
    fireEvent.click(rawTab);
    expect(rawTab.className).toContain("active");
  });

  it("renders placeholders for <required> and NULL", () => {
    const { rerender } = render(
      <ValueInspectorPanel
        target={{
          ...baseTarget,
          value: null,
          insert: {
            tabId: "t1",
            insertId: "i1",
            usingDefault: false,
            explicitNull: false,
            unfilledRequired: true,
          },
        }}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText("<required>")).toBeDefined();

    rerender(
      <ValueInspectorPanel
        target={{
          ...baseTarget,
          value: null,
        }}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText("NULL")).toBeDefined();
  });
});
