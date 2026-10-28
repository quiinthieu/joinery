import { expect, test } from "vitest";
import { CompletionWorkerIndex, type CompletionRequest } from "./completion-protocol";

test("responses report the installed revision and reject obsolete requests", () => {
  const worker = new CompletionWorkerIndex();
  worker.handle({
    type: "replace",
    section: "catalog",
    revision: 2,
    items: [{ label: "orders", path: ["orders"], type: "type" }],
  });
  const request: CompletionRequest = {
    type: "prefix",
    id: 1,
    revision: 1,
    stamp: { document: 1, context: 1, position: 2 },
    parts: [{ value: "or", quoted: false }],
    engine: "postgres",
    columnsOnly: false,
  };
  expect(worker.handle(request)).toMatchObject({ type: "suggestions", revision: 2, items: [] });
  expect(worker.handle({ ...request, revision: 2 })).toMatchObject({
    items: [{ label: "orders" }],
  });
  worker.handle({
    type: "replace",
    section: "columns",
    revision: 3,
    items: [{ label: "order_id", type: "property" }],
  });
  expect(worker.handle({ ...request, revision: 3 })).toMatchObject({
    items: [{ label: "order_id" }, { label: "orders" }],
  });
  worker.handle({ type: "replace", section: "catalog", revision: 4, items: [] });
  worker.handle({
    type: "replace",
    section: "columns",
    revision: 3,
    items: [{ label: "old", type: "property" }],
  });
  expect(
    worker.handle({ ...request, revision: 4, parts: [{ value: "", quoted: false }] }),
  ).toMatchObject({ revision: 4, items: [] });
});

test("quoted segments preserve case, embedded dots and qualification identity", () => {
  const worker = new CompletionWorkerIndex();
  const paths = [
    ["a.b", "Orders"],
    ["a", "b", "orders"],
    ["a.b", "orders"],
  ];
  worker.handle({
    type: "replace",
    section: "catalog",
    revision: 1,
    items: paths.map((path) => ({ path, label: path.join("."), type: "type" })),
  });
  const request: CompletionRequest = {
    type: "prefix",
    id: 1,
    revision: 1,
    stamp: { document: 1, context: 1, position: 2 },
    parts: [
      { value: "a.b", quoted: true },
      { value: "O", quoted: true },
    ],
    engine: "postgres",
    columnsOnly: false,
  };
  expect(worker.handle(request)).toMatchObject({ items: [{ path: ["a.b", "Orders"] }] });
  expect(
    worker.handle({
      ...request,
      parts: [
        { value: "a", quoted: false },
        { value: "b", quoted: false },
        { value: "or", quoted: false },
      ],
    }),
  ).toMatchObject({ items: [{ path: ["a", "b", "orders"] }] });
  expect(worker.handle({ ...request, columnsOnly: true })).toMatchObject({ items: [] });
});

test("caps combined catalog and column results at 100", () => {
  const worker = new CompletionWorkerIndex();
  const items = Array.from({ length: 200 }, (_, i) => ({ label: `name${i}`, type: "property" }));
  worker.handle({ type: "replace", section: "catalog", revision: 1, items });
  worker.handle({ type: "replace", section: "columns", revision: 2, items });
  const result = worker.handle({
    type: "prefix",
    revision: 2,
    id: 1,
    stamp: { document: 1, context: 1, position: 0 },
    parts: [{ value: "", quoted: false }],
    engine: "postgres",
    columnsOnly: false,
  });
  expect(result.type === "suggestions" && result.items.length).toBe(100);
});
