import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";

describe("application shell", () => {
  it("renders the product without waiting for a database", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: "Welcome to Joinery." })).toBeTruthy();
  });
});
