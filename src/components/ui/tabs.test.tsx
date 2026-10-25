import { afterEach, expect, test } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { tabNavigation } from "./tabs";

afterEach(cleanup);

test("tab arrows wrap, Home/End select, and auxiliary controls keep their keys", () => {
  const selected: string[] = [];
  const view = render(
    <div role="tablist" onKeyDown={tabNavigation}>
      {["one", "two", "three"].map((name) => (
        <div key={name}>
          <button role="tab" onClick={() => selected.push(name)}>
            {name}
          </button>
          <button>Close {name}</button>
        </div>
      ))}
      <button role="tab" disabled>
        Disabled
      </button>
    </div>,
  );
  const tabs = view.getAllByRole("tab");
  tabs[0].focus();
  fireEvent.keyDown(tabs[0], { key: "ArrowLeft" });
  expect(document.activeElement).toBe(tabs[2]);
  fireEvent.keyDown(tabs[2], { key: "ArrowRight" });
  expect(document.activeElement).toBe(tabs[0]);
  fireEvent.keyDown(tabs[0], { key: "End" });
  fireEvent.keyDown(tabs[2], { key: "Home" });
  expect(selected).toEqual(["three", "one", "three", "one"]);
  const close = view.getByRole("button", { name: "Close one" });
  close.focus();
  fireEvent.keyDown(close, { key: "ArrowRight" });
  expect(document.activeElement).toBe(close);
  expect(selected).toHaveLength(4);
});
