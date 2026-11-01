import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { ExecutionTiming } from "./ExecutionTiming";
afterEach(cleanup);
test("missing timings remain unavailable while measured zeros remain visible", () => {
  const { rerender } = render(<ExecutionTiming clientElapsedMs={0} />);
  expect(screen.getAllByText("Unavailable")).toHaveLength(4);
  expect(screen.getAllByText("0 ms")).toHaveLength(1);
  rerender(
    <ExecutionTiming
      clientElapsedMs={0}
      metrics={{
        type: "runMetrics",
        runId: "r",
        queueMs: 0,
        activeMs: 0,
        totalMs: 0,
        firstRowMs: null,
      }}
    />,
  );
  expect(screen.getAllByText("Unavailable")).toHaveLength(1);
  expect(screen.getAllByText("0 ms")).toHaveLength(4);
});
