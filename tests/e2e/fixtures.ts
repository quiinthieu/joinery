import { expect, test as base } from "@playwright/test";
import type { APIRequestContext } from "@playwright/test";

const BRIDGE = "http://127.0.0.1:1422";
const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };

async function disconnectFixtures(request: APIRequestContext) {
  const response = await request.post(`${BRIDGE}/rpc/disconnect_fixtures`, {
    headers,
    data: {},
  });
  expect(response.ok(), "disconnect_fixtures should return a successful HTTP response").toBe(true);
  const payload = (await response.json()) as { error?: unknown };
  expect(payload.error, "disconnect_fixtures should not return an RPC error").toBeUndefined();
}

export const test = base.extend<{ harnessIsolation: void }>({
  harnessIsolation: [
    async ({ request }, use) => {
      await disconnectFixtures(request);
      try {
        await use();
      } finally {
        await disconnectFixtures(request);
      }
    },
    { auto: true },
  ],
});

export { expect };
export type { APIRequestContext, Locator, Page, Route } from "@playwright/test";
