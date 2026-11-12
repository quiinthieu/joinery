import { describe, expect, it, vi } from "vitest";
import type { RecoveryBackup } from "./ipc/generated";

vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: vi.fn() }));
vi.mock("./ipc/client", () => ({ appErrorOf: vi.fn(), client: {}, messageOf: vi.fn() }));
vi.mock("./state/studio", () => ({ useStudio: {} }));

import { sameRecoveryInventory } from "./native-encrypted-recovery";

const inventory: RecoveryBackup[] = [
  {
    metadata: {
      id: "19f006d7-519d-42ec-b9fc-f66610864082",
      createdAt: 1794501016793,
      sizeBytes: 106857,
      digest: "54277425a01c68f4e20506da65d72286b29676d8ebfd1a4b3daf3291e52c0f5e",
    },
    format: "encryptedV1",
  },
  {
    metadata: {
      id: "c8be341b-4b02-44ed-be8c-ad5790e67b53",
      createdAt: 1794501014928,
      sizeBytes: 106857,
      digest: "332430331bb936a5ac84a21e17124012c1b96b5e94116fb7f4a58fcb96a16ea9",
    },
    format: "encryptedV1",
  },
];

describe("native encrypted recovery inventory", () => {
  it("accepts serde_json::Value key ordering without weakening backup identity", () => {
    // native_smoke_report and checkpoint roundtrip through sorted Value maps,
    // while recovery_backups serializes Rust structs in declaration order.
    const persisted = inventory.map(({ metadata, format }) => ({
      format,
      metadata: {
        createdAt: metadata.createdAt,
        digest: metadata.digest,
        id: metadata.id,
        sizeBytes: metadata.sizeBytes,
      },
    }));
    expect(JSON.stringify(inventory)).not.toBe(JSON.stringify(persisted));
    expect(sameRecoveryInventory(inventory, persisted)).toBe(true);
  });

  it.each(["format", "id", "createdAt", "sizeBytes", "digest"] as const)(
    "rejects a changed %s in either backup",
    (field) => {
      for (const index of [0, 1]) {
        const changed = structuredClone(inventory);
        if (field === "format") changed[index].format = "legacySqlite";
        else if (field === "createdAt" || field === "sizeBytes") changed[index].metadata[field]++;
        else changed[index].metadata[field] = "changed";
        expect(sameRecoveryInventory(changed, inventory)).toBe(false);
      }
    },
  );

  it("rejects removed, added, duplicated, or reordered backup identities", () => {
    for (const changed of [
      inventory.slice(0, 1),
      [...inventory, inventory[0]],
      [inventory[0], inventory[0]],
      [...inventory].reverse(),
    ]) {
      expect(sameRecoveryInventory(changed, inventory)).toBe(false);
    }
    expect(sameRecoveryInventory(inventory, inventory)).toBe(true);
  });
});
