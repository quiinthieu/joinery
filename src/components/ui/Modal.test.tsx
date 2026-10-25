import { afterEach, expect, test, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { Modal } from "./Modal";

afterEach(cleanup);

test("a dialog hides background content from assistive navigation and restores it", () => {
  const view = render(
    <div>
      <main aria-hidden="false">
        <button>Background action</button>
      </main>
      <Modal title="Settings" subtitle="Private settings" onClose={vi.fn()}>
        <input aria-label="Name" />
      </Modal>
    </div>,
  );
  const main = view.container.querySelector("main")!;
  expect(main.hasAttribute("inert")).toBe(true);
  expect(main.getAttribute("aria-hidden")).toBe("true");
  const dialog = view.getByRole("dialog");
  const description = dialog.getAttribute("aria-describedby");
  expect(description).toBeTruthy();
  expect(document.getElementById(description!)?.textContent).toBe("Private settings");
  view.rerender(
    <div>
      <main aria-hidden="false">
        <button>Background action</button>
      </main>
    </div>,
  );
  expect(main.hasAttribute("inert")).toBe(false);
  expect(main.getAttribute("aria-hidden")).toBe("false");
});
