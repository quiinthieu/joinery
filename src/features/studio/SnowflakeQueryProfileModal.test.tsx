import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { client } from "../../ipc/client";
import type { SnowflakeOperatorProfile, SnowflakeQueryHistoryEntry } from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { SnowflakeQueryProfileModal } from "./SnowflakeQueryProfileModal";

describe("SnowflakeQueryProfileModal", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useStudio.setState({
      sessions: {},
      sessionStates: {},
      snowflakeContextAuthorities: {},
    });
  });

  it("loads session QUERY_HISTORY with privacy redaction and inspects GET_QUERY_OPERATOR_STATS bottlenecks and spills", async () => {
    const historyRows: SnowflakeQueryHistoryEntry[] = [
      {
        queryId: "01b4a9c0-0001-9f12-0000-00420001a111",
        queryText: "[SQL text hidden by connection privacy policy]",
        queryType: "SELECT",
        executionStatus: "SUCCESS",
        warehouseName: "ANALYTICS_WH",
        warehouseSize: "Small",
        databaseName: "PROD_DB",
        schemaName: "PUBLIC",
        startTime: "2026-11-16 08:00:00Z",
        totalElapsedMs: 1420,
        compilationMs: 120,
        executionMs: 1280,
        queuedMs: 20,
        bytesScanned: 1048576,
        cacheHitRatio: 0.75,
        bytesSpilledLocal: 524288,
        bytesSpilledRemote: 0,
        partitionsScanned: 4,
        partitionsTotal: 16,
        rowsProduced: 2500,
        errorMessage: null,
      },
    ];

    const opProfile: SnowflakeOperatorProfile = {
      queryId: "01b4a9c0-0001-9f12-0000-00420001a111",
      totalSpilledLocalBytes: 524288,
      totalSpilledRemoteBytes: 0,
      bottleneckStepId: 2,
      bottleneckOperatorId: 2,
      operators: [
        {
          stepId: 1,
          operatorId: 2,
          parentOperators: [],
          operatorType: "Result",
          overallPercentage: 0.08,
          inputRows: 2500,
          outputRows: null,
          bytesScanned: null,
          cacheHitRatio: null,
          partitionsScanned: null,
          partitionsTotal: null,
          bytesSpilledLocal: null,
          bytesSpilledRemote: null,
          detail: null,
        },
        {
          stepId: 2,
          operatorId: 2,
          parentOperators: [1],
          operatorType: "TableScan",
          overallPercentage: 0.92,
          inputRows: null,
          outputRows: 2500,
          bytesScanned: 1048576,
          cacheHitRatio: 0.75,
          partitionsScanned: 4,
          partitionsTotal: 16,
          bytesSpilledLocal: 524288,
          bytesSpilledRemote: 0,
          detail: "PROD_DB.PUBLIC.ORDERS",
        },
      ],
    };

    const callSpy = vi.spyOn(client, "call").mockImplementation(async (cmd, args: unknown) => {
      if (cmd === "snowflake_query_history") {
        return historyRows as never;
      }
      if (cmd === "snowflake_query_operator_profile") {
        const req = (args as { request: { queryId: string } }).request;
        return { ...opProfile, queryId: req.queryId } as never;
      }
      throw new Error(`Unexpected IPC call: ${cmd}`);
    });

    const onClose = vi.fn();
    render(<SnowflakeQueryProfileModal sessionId="sf-sess-1" onClose={onClose} />);

    await waitFor(() => {
      expect(screen.getByText("[SQL text hidden by connection privacy policy]")).toBeTruthy();
    });
    expect(screen.getByText("ANALYTICS_WH (Small)")).toBeTruthy();
    expect(screen.getAllByText("4 / 16").length).toBeGreaterThan(0);

    await waitFor(() => {
      expect(screen.getByTestId("snowflake-operator-2-2")).toBeTruthy();
    });
    // Only step 2 operator 2 gets the Bottleneck badge, NOT step 1 operator 2!
    expect(screen.getAllByText("Bottleneck")).toHaveLength(1);
    expect(screen.getByTestId("snowflake-operator-2").textContent).not.toContain("Bottleneck");
    expect(screen.getByTestId("snowflake-operator-2-2").textContent).toContain("Bottleneck");
    expect(screen.getByText("PROD_DB.PUBLIC.ORDERS")).toBeTruthy();
    expect(screen.getByText("#2 TableScan (92.0%)")).toBeTruthy();

    // Custom query ID lookup
    const input = screen.getByLabelText("Inspect Snowflake query ID");
    fireEvent.change(input, { target: { value: "01b4a9c0-0001-9f12-0000-00999999b222" } });
    fireEvent.click(screen.getByRole("button", { name: /Inspect profile/i }));

    await waitFor(() => {
      expect(callSpy).toHaveBeenCalledWith("snowflake_query_operator_profile", {
        request: {
          sessionId: "sf-sess-1",
          queryId: "01b4a9c0-0001-9f12-0000-00999999b222",
        },
      });
    });

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
