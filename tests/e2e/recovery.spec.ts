import { test, expect } from "./fixtures";
import { randomUUID } from "node:crypto";
import type { APIRequestContext, Locator, Page } from "./fixtures";
import type { Bootstrap, RecoveryBackup, RecoveryOperation } from "../../src/ipc/generated";

const BRIDGE = "http://127.0.0.1:1422";
const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
const key = "disposable encrypted recovery passphrase";
const marker = "joinery.metadataRecoveryOperation.v1";

async function rpc<T>(
  request: APIRequestContext,
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  const response = await request.post(`${BRIDGE}/rpc/${command}`, { headers, data: args });
  expect(response.ok()).toBe(true);
  const payload = (await response.json()) as { result: T; error?: { message: string } };
  expect(payload.error, `${command} must succeed in the disposable harness`).toBeUndefined();
  return payload.result;
}

async function fillKey(recovery: Locator, value = key) {
  await recovery.getByLabel("Backup passphrase", { exact: true }).fill(value);
  await recovery.getByLabel("Confirm backup passphrase", { exact: true }).fill(value);
}

async function settled(page: Page, recovery: Locator, notice: string | RegExp) {
  // The operation ID is removed only after native terminal status, inventory
  // refresh and any required restored-bootstrap refresh have all completed.
  await expect(recovery.getByText(notice, { exact: typeof notice === "string" })).toBeVisible({
    timeout: 60000,
  });
  await expect
    .poll(() => page.evaluate((name) => sessionStorage.getItem(name), marker), { timeout: 10000 })
    .toBeNull();
}

test("encrypted metadata recovery exports, imports, preserves wrong-key state, restores and undoes without replay", async ({
  page,
  request,
}) => {
  test.setTimeout(180000);
  const initial = await rpc<RecoveryBackup[]>(request, "recovery_backups");
  const initialBootstrap = await rpc<Bootstrap>(request, "bootstrap");
  expect(
    initialBootstrap.passphraseConfigured,
    "the disposable fixture must not require an unknown owner write key",
  ).toBe(false);
  const starts: { id: string; kind: string }[] = [];
  const lookups: string[] = [];
  const remoteCommands: string[] = [];
  page.on("request", (request) => {
    if (request.method() !== "POST" || !request.url().startsWith(`${BRIDGE}/rpc/`)) return;
    const command = request.url().split("/").at(-1)!;
    if (
      [
        "connect",
        "connect_prepared",
        "execute_plan",
        "start_snowflake_console",
        "start_export",
      ].includes(command)
    )
      remoteCommands.push(command);
    if (command === "start_metadata_recovery") {
      const args = request.postDataJSON();
      starts.push({ id: args.id, kind: args.request.kind });
    }
    if (command === "metadata_recovery_operation") lookups.push(request.postDataJSON().id);
  });
  // Drop precisely one response after the real native command admitted its
  // original operation. The UI must recover that ID by lookup, not start again.
  let dropped = false;
  await page.route(`${BRIDGE}/rpc/start_metadata_recovery`, async (route) => {
    if (dropped) {
      await route.continue();
      return;
    }
    const response = await route.fetch();
    expect(response.ok()).toBe(true);
    const payload = (await response.json()) as { result?: RecoveryOperation; error?: unknown };
    expect(payload.error).toBeUndefined();
    expect(payload.result?.id).toBe(route.request().postDataJSON().id);
    expect(payload.result?.kind).toBe("create");
    dropped = true;
    await route.abort("connectionreset");
  });

  await page.goto("/");
  await page.getByRole("button", { name: /^Preferences/ }).click();
  const recovery = page.getByRole("region", { name: "Local metadata recovery" });
  await expect(recovery).toContainText("Live metadata and temporary staging remain plaintext");
  await expect(
    recovery.getByRole("button", { name: "Create metadata backup", exact: true }),
  ).toBeDisabled();
  await fillKey(recovery);
  await recovery.getByRole("button", { name: "Create metadata backup", exact: true }).click();
  await settled(page, recovery, "Encrypted backup created.");
  expect(dropped).toBe(true);
  expect(starts).toHaveLength(1);
  expect(lookups).toContain(starts[0].id);
  await expect(recovery.getByLabel("Backup passphrase", { exact: true })).toHaveValue("");
  await expect(recovery.getByLabel("Confirm backup passphrase", { exact: true })).toHaveValue("");
  const createdOperation = await rpc<RecoveryOperation>(request, "metadata_recovery_operation", {
    id: starts[0].id,
  });
  expect(createdOperation.state).toBe("succeeded");
  const original = createdOperation.backup!;
  expect(original.format).toBe("encryptedV1");
  expect(initial.some((entry) => entry.metadata.id === original.metadata.id)).toBe(false);
  const snapshots = recovery.getByLabel("Metadata backup", { exact: true });
  await snapshots.selectOption(original.metadata.id);

  const exportedName = `metadata-recovery-${Date.now()}.age`;
  await rpc(request, "test_queue_save_file", { fileName: exportedName });
  await recovery.getByRole("button", { name: "Export encrypted backup…", exact: true }).click();
  await settled(page, recovery, "Encrypted backup exported.");
  const exported = await rpc<{ bytes: number; sha256: string }>(
    request,
    "test_sandbox_file_summary",
    { fileName: exportedName },
  );
  expect(exported.bytes).toBe(original.metadata.sizeBytes);
  expect(exported.sha256).toBe(original.metadata.digest);

  await rpc(request, "test_queue_open_file", { fileName: exportedName });
  await recovery.getByRole("button", { name: "Import encrypted backup…", exact: true }).click();
  await settled(page, recovery, "Encrypted backup imported.");
  const importedOperation = await rpc<RecoveryOperation>(request, "metadata_recovery_operation", {
    id: starts.at(-1)!.id,
  });
  expect(importedOperation.state).toBe("succeeded");
  const imported = importedOperation.backup!;
  expect(imported.format).toBe("encryptedV1");
  expect(imported.metadata.id).not.toBe(original.metadata.id);
  expect(imported.metadata.digest).toBe(exported.sha256);
  expect(imported.metadata.sizeBytes).toBe(exported.bytes);
  expect(await rpc(request, "test_sandbox_file_summary", { fileName: exportedName })).toEqual(
    exported,
  );

  // This local mutation must survive a rejected decryption, disappear after a
  // successful restore of the imported copy, then return from encrypted undo.
  const group = `Restore fixture ${Date.now()}`;
  await page.getByLabel("Group name", { exact: true }).fill(group);
  await page.getByLabel("Parent group", { exact: true }).selectOption("local");
  await page.getByRole("button", { name: "Add group", exact: true }).click();
  const groupButton = page.getByRole("button", { name: `Rename group ${group}`, exact: true });
  await expect(groupButton).toBeVisible();
  const beforeWrongKey = await rpc<Bootstrap>(request, "bootstrap");
  const inventoryBeforeWrongKey = await rpc<RecoveryBackup[]>(request, "recovery_backups");
  await snapshots.selectOption(imported.metadata.id);
  await fillKey(recovery, "incorrect disposable backup passphrase");
  await recovery.getByLabel("Type RESTORE to replace metadata", { exact: true }).fill("RESTORE");
  await recovery.getByRole("button", { name: "Restore metadata", exact: true }).click();
  await settled(page, recovery, "Recovery operation failed; no operation was retried.");
  await expect(recovery).toContainText("Could not decrypt the metadata backup");
  await expect(groupButton).toBeVisible();
  const rejected = await rpc<RecoveryOperation>(request, "metadata_recovery_operation", {
    id: starts.at(-1)!.id,
  });
  expect(rejected.state).toBe("failed");
  expect(rejected.backup).toBeNull();
  expect((await rpc<Bootstrap>(request, "bootstrap")).groups).toEqual(beforeWrongKey.groups);
  expect((await rpc<Bootstrap>(request, "bootstrap")).profiles).toEqual(beforeWrongKey.profiles);
  expect(await rpc(request, "recovery_backups")).toEqual(inventoryBeforeWrongKey);

  await snapshots.selectOption(imported.metadata.id);
  await fillKey(recovery);
  await recovery.getByLabel("Type RESTORE to replace metadata", { exact: true }).fill("RESTORE");
  await recovery.getByRole("button", { name: "Restore metadata", exact: true }).click();
  await settled(page, recovery, /Metadata restored\. Encrypted undo backup:/);
  await expect(groupButton).toHaveCount(0);
  const restored = await rpc<RecoveryOperation>(request, "metadata_recovery_operation", {
    id: starts.at(-1)!.id,
  });
  expect(restored.state).toBe("succeeded");
  const undo = restored.backup!;
  expect(undo.format).toBe("encryptedV1");
  expect(undo.metadata.id).not.toBe(imported.metadata.id);
  await snapshots.selectOption(undo.metadata.id);
  await fillKey(recovery);
  await recovery.getByLabel("Type RESTORE to replace metadata", { exact: true }).fill("RESTORE");
  await recovery.getByRole("button", { name: "Restore metadata", exact: true }).click();
  await settled(page, recovery, /Metadata restored\. Encrypted undo backup:/);
  await expect(groupButton).toBeVisible();
  const undone = await rpc<RecoveryOperation>(request, "metadata_recovery_operation", {
    id: starts.at(-1)!.id,
  });
  expect(undone.state).toBe("succeeded");

  // Remove only copies this test created; unrelated pre-existing inventory stays.
  for (const entry of [original, imported, undo, undone.backup!]) {
    await snapshots.selectOption(entry.metadata.id);
    await recovery.getByLabel("Type RESTORE to replace metadata", { exact: true }).fill("DELETE");
    await recovery.getByRole("button", { name: "Delete snapshot", exact: true }).click();
    await expect(snapshots.locator(`option[value="${entry.metadata.id}"]`)).toHaveCount(0);
  }
  expect(await rpc(request, "recovery_backups")).toEqual(initial);
  expect(starts.map((entry) => entry.kind)).toEqual([
    "create",
    "export",
    "import",
    "restore",
    "restore",
    "restore",
  ]);
  expect(new Set(starts.map((entry) => entry.id)).size).toBe(starts.length);
  expect(remoteCommands, "metadata recovery must never connect or dispatch remote work").toEqual(
    [],
  );
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page.reload();
  await page.getByRole("button", { name: /^Preferences/ }).click();
  await expect(groupButton).toBeVisible();
  await expect.poll(() => page.evaluate((name) => sessionStorage.getItem(name), marker)).toBeNull();
  expect(starts).toHaveLength(6);
});

test("a retained recovery ID gates startup editing until native reconciliation and bootstrap complete", async ({
  page,
}) => {
  const retained = randomUUID();
  const remoteCommands: string[] = [];
  const reconciliations: string[] = [];
  let reconciledBootstrapHeld = false;
  let releaseBootstrap!: () => void;
  const heldBootstrap = new Promise<void>((resolve) => {
    releaseBootstrap = resolve;
  });
  await page.addInitScript(({ key, id }) => sessionStorage.setItem(key, id), {
    key: marker,
    id: retained,
  });
  page.on("request", (request) => {
    if (request.method() !== "POST" || !request.url().startsWith(`${BRIDGE}/rpc/`)) return;
    const command = request.url().split("/").at(-1)!;
    if (
      [
        "connect",
        "connect_prepared",
        "execute_plan",
        "start_snowflake_console",
        "start_export",
        "start_metadata_recovery",
      ].includes(command)
    )
      remoteCommands.push(command);
    if (command === "reconcile_metadata_recovery") reconciliations.push(request.postDataJSON().id);
  });
  await page.route(`${BRIDGE}/rpc/bootstrap`, async (route) => {
    const response = await route.fetch();
    if (reconciliations.includes(retained)) {
      reconciledBootstrapHeld = true;
      await heldBootstrap;
    }
    await route.fulfill({ response });
  });
  await page.goto("/");
  const gate = page.getByRole("dialog", { name: "Resolve metadata recovery", exact: true });
  await expect(gate).toBeVisible();
  await expect(page.getByRole("button", { name: "New query tab", exact: true })).toHaveCount(0);
  await page.keyboard.press("Meta+t");
  await expect(page.getByRole("button", { name: "New query tab", exact: true })).toHaveCount(0);
  await gate
    .getByRole("button", { name: "Reload native metadata without retrying", exact: true })
    .click();
  await expect.poll(() => reconciledBootstrapHeld).toBe(true);
  await expect(gate).toBeVisible();
  expect(await page.evaluate((key) => sessionStorage.getItem(key), marker)).toBe(retained);
  expect(reconciliations).toEqual([retained]);
  releaseBootstrap();
  await expect(gate).toHaveCount(0);
  await expect(page.getByRole("button", { name: "New query tab", exact: true })).toBeVisible();
  expect(await page.evaluate((key) => sessionStorage.getItem(key), marker)).toBeNull();
  expect(remoteCommands).toEqual([]);
});
