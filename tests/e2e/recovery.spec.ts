import { test, expect } from "@playwright/test";

test("local metadata can be backed up, restored, undone and deleted", async ({ page, request }) => {
  const response = await request.post("http://127.0.0.1:1422/rpc/disconnect_fixtures", {
    headers: { Authorization: "Bearer joinery-disposable-browser-tests-only" },
    data: {},
  });
  expect(response.ok()).toBe(true);
  await page.goto("/");
  await page.getByRole("button", { name: /^Preferences/ }).click();
  const recovery = page.getByRole("region", { name: "Local metadata recovery" });
  await recovery.getByRole("button", { name: "Create metadata backup", exact: true }).click();
  await expect(recovery.getByRole("status")).toContainText("snapshot created");
  const snapshots = recovery.getByLabel("Metadata backup", { exact: true });
  const id = await snapshots.locator("option").nth(1).getAttribute("value");
  expect(id).toBeTruthy();
  const group = `Restore fixture ${Date.now()}`;
  await page.getByLabel("Group name", { exact: true }).fill(group);
  await page.getByLabel("Parent group", { exact: true }).selectOption("local");
  await page.getByRole("button", { name: "Add group", exact: true }).click();
  await expect(
    page.getByRole("button", { name: `Rename group ${group}`, exact: true }),
  ).toBeVisible();
  await snapshots.selectOption(id!);
  await expect(
    recovery.getByRole("button", { name: "Restore metadata", exact: true }),
  ).toBeDisabled();
  await recovery.getByLabel("Type RESTORE to replace metadata").fill("RESTORE");
  await recovery.getByRole("button", { name: "Restore metadata", exact: true }).click();
  await expect(recovery.getByRole("status")).toContainText("Metadata restored");
  await expect(
    page.getByRole("button", { name: `Rename group ${group}`, exact: true }),
  ).toHaveCount(0);
  const undoId = await snapshots.locator("option").nth(1).getAttribute("value");
  expect(undoId).not.toBe(id);
  await snapshots.selectOption(undoId!);
  await recovery.getByLabel("Type RESTORE to replace metadata").fill("RESTORE");
  await recovery.getByRole("button", { name: "Restore metadata", exact: true }).click();
  await expect(recovery.getByRole("status")).toContainText("Metadata restored");
  await expect(
    page.getByRole("button", { name: `Rename group ${group}`, exact: true }),
  ).toBeVisible();
  await snapshots.selectOption(id!);
  await recovery.getByLabel("Type RESTORE to replace metadata").fill("DELETE");
  await recovery.getByRole("button", { name: "Delete snapshot", exact: true }).click();
  await expect(recovery.getByRole("status")).toContainText("Snapshot deleted");
  await expect(snapshots.locator(`option[value="${id}"]`)).toHaveCount(0);
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page.reload();
  await page.getByRole("button", { name: /^Preferences/ }).click();
  await expect(
    page.getByRole("button", { name: `Rename group ${group}`, exact: true }),
  ).toBeVisible();
});
