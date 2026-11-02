import { afterEach, describe, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import {
  ParameterDialog,
  clearTabParamDrafts,
  getTabParamDrafts,
  setTabParamDrafts,
  pruneTabParamDrafts,
  buildQueryParamInputs,
} from "./ParameterDialog";
import { ReviewDialog } from "../safety/ReviewDialog";
import type { PreparedPlan, QueryParamInput, QueryParamSlot } from "../../ipc/generated";

afterEach(cleanup);

describe("ParameterDialog & ReviewDialog bound parameters", () => {
  test("handles prototype-shaped names and drops drafts for closed tabs", () => {
    const slots: QueryParamSlot[] = ["constructor", "__proto__"].map((key) => ({
      key,
      positional: false,
      occurrences: 1,
    }));
    expect(buildQueryParamInputs(slots, {}).inputs).toEqual(
      slots.map(({ key }) => ({ key, kind: "text", value: "" })),
    );
    const onRun = vi.fn(async (_inputs: QueryParamInput[]) => {});
    const view = render(
      <ParameterDialog slots={slots} onSaveDrafts={vi.fn()} onRun={onRun} onClose={vi.fn()} />,
    );
    fireEvent.change(view.getByLabelText("Value for parameter __proto__"), {
      target: { value: "bound text" },
    });
    fireEvent.click(view.getByRole("button", { name: "Run query" }));
    expect(onRun.mock.calls[0]?.[0]?.[1]?.value).toBe("bound text");

    setTabParamDrafts("closed-tab", {
      secret: { kind: "text", value: "ephemeral", isNull: false },
    });
    setTabParamDrafts("open-tab", { id: { kind: "integer", value: "42", isNull: false } });
    pruneTabParamDrafts(["open-tab"]);
    expect(getTabParamDrafts("closed-tab")).toEqual({});
    expect(getTabParamDrafts("open-tab").id?.value).toBe("42");
    clearTabParamDrafts("open-tab");
  });

  test("validates typed inputs, handles NULL toggles, and keeps drafts transient in memory", async () => {
    const slots: QueryParamSlot[] = [
      { key: "customer_id", positional: false, occurrences: 2 },
      { key: "$1", positional: true, occurrences: 1 },
      { key: "deleted_at", positional: false, occurrences: 1 },
    ];
    const onSaveDrafts = vi.fn((drafts) => setTabParamDrafts("tab-param-1", drafts));
    const onRun = vi.fn(async (_inputs: QueryParamInput[]) => {});

    const view = render(
      <ParameterDialog
        slots={slots}
        initialDrafts={getTabParamDrafts("tab-param-1")}
        onSaveDrafts={onSaveDrafts}
        onRun={onRun}
        onClose={vi.fn()}
      />,
    );

    expect(view.getByText(":customer_id")).toBeTruthy();
    expect(view.getByText("×2")).toBeTruthy();
    expect(view.getByText("$1")).toBeTruthy();

    fireEvent.change(view.getByLabelText("Type for parameter customer_id"), {
      target: { value: "integer" },
    });
    fireEvent.change(view.getByLabelText("Value for parameter customer_id"), {
      target: { value: "not-an-int" },
    });
    fireEvent.click(view.getByRole("button", { name: "Run query" }));

    expect(view.getByText("Parameter :customer_id must be a 64-bit integer")).toBeTruthy();
    expect(onRun).not.toHaveBeenCalled();

    fireEvent.change(view.getByLabelText("Value for parameter customer_id"), {
      target: { value: "42" },
    });
    fireEvent.change(view.getByLabelText("Type for parameter $1"), {
      target: { value: "json" },
    });
    fireEvent.change(view.getByLabelText("Value for parameter $1"), {
      target: { value: '{"tier":"pro"}' },
    });
    fireEvent.change(view.getByLabelText("Type for parameter deleted_at"), {
      target: { value: "timestamp" },
    });
    fireEvent.click(view.getByLabelText("NULL for parameter deleted_at"));

    fireEvent.click(view.getByRole("button", { name: "Run query" }));

    await waitFor(() => expect(onRun).toHaveBeenCalledTimes(1));
    expect(onRun.mock.calls[0]?.[0]).toEqual([
      { key: "customer_id", kind: "integer", value: "42" },
      { key: "$1", kind: "json", value: '{"tier":"pro"}' },
      { key: "deleted_at", kind: "timestamp", value: null },
    ]);

    expect(getTabParamDrafts("tab-param-1").customer_id?.value).toBe("42");
    clearTabParamDrafts("tab-param-1");
    expect(getTabParamDrafts("tab-param-1")).toEqual({});
  });

  test("ReviewDialog displays bound parameters and NULL markers in exact batch review", () => {
    const plan: PreparedPlan = {
      id: "plan-1",
      sessionId: "sess-1",
      statements: [
        {
          sql: "UPDATE orders SET note = :note, closed_at = $1 WHERE id = :id",
          kind: "write",
          missingWhere: false,
        },
      ],
      boundParameters: [
        { key: "note", kind: "text", value: "Reviewed value" },
        { key: "$1", kind: "timestamp", value: null },
        { key: "id", kind: "integer", value: "101" },
      ],
      requiresApproval: true,
      requiresPassphrase: false,
      confirmationText: null,
      warnings: [],
      expiresAt: Date.now() + 60_000,
    };

    const view = render(
      <ReviewDialog plan={plan} onExecute={vi.fn(async () => {})} onClose={vi.fn()} />,
    );

    expect(view.getByText("Bound parameters (3)")).toBeTruthy();
    expect(view.getByText('"Reviewed value"')).toBeTruthy();
    expect(view.getByText("NULL")).toBeTruthy();
    expect(view.getByText('"101"')).toBeTruthy();
  });
});
