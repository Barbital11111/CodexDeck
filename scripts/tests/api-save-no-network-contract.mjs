import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const controllerPath = path.resolve(testDir, "..", "..", "src", "hooks", "useCodexController.ts");
const controller = fs.readFileSync(controllerPath, "utf8");

test("API account saves do not test notification providers", () => {
  assert.doesNotMatch(controller, /probeNotificationProviderForImport/);
  assert.doesNotMatch(controller, /test_notification_provider/);
});
