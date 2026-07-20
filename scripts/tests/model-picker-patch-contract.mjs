import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const patchScriptPath = path.resolve(testDir, "..", "patch-codex-model-picker.mjs");
const patchScript = fs.readFileSync(patchScriptPath, "utf8");

function stringConstant(name) {
  const match = patchScript.match(
    new RegExp(`const ${name}\\s*=\\s*("(?:\\\\.|[^"\\\\])*");`),
  );
  assert.ok(match, `missing string constant ${name}`);
  return JSON.parse(match[1]);
}

function compilePowerFilter(source, bindingName, selections) {
  return new Function(bindingName, `${source};return K;`)(selections);
}

function managedSelection(model, modelLabel = "fallback") {
  return {
    id: `${model}:max`,
    model,
    modelLabel,
    reasoningEffort: "max",
  };
}

for (const [name, bindingName] of [
  ["CURRENT_LEGACY_POWER_SELECTION_FILTER", "q"],
  ["CURRENT_FALLBACK_POWER_SELECTION_FILTER", "ce"],
]) {
  test(`${name} uses the provider displayName in collapsed selections`, () => {
    const model = "gpt-5.6-luna";
    const selections = [managedSelection(model)];
    const filter = compilePowerFilter(stringConstant(name), bindingName, selections);

    const result = filter([{ model, displayName: "GPT 5.6 Luna" }]);

    assert.equal(result.length, 1);
    assert.equal(result[0].modelLabel, "GPT 5.6 Luna");
  });
}

test("collapsed reasoning labels use the agreed English six-level vocabulary", () => {
  const labels = JSON.parse(stringConstant("COLLAPSED_REASONING_LABELS_JSON"));

  assert.deepEqual(labels, {
    low: "Low",
    medium: "Medium",
    high: "High",
    xhigh: "Extra high",
    max: "Max",
    ultra: "ULTRA",
  });
  assert.match(patchScript, /replaceCollapsedPowerSelectionLabel/);
});

test("the simplified Chinese collapsed Max label is patched to English", () => {
  assert.equal(
    stringConstant("ZH_CN_MAX_LABEL_BEFORE"),
    '"composer.mode.local.reasoning.max.label":`最高`',
  );
  assert.equal(
    stringConstant("ZH_CN_MAX_LABEL_AFTER"),
    '"composer.mode.local.reasoning.max.label":`Max`',
  );
  assert.match(patchScript, /replaceZhCnReasoningLabels/);
  assert.match(patchScript, /"model-picker-v23"/);
});

test("stable metadata stops retrying only for permanent environment errors", () => {
  const before = stringConstant("STABLE_METADATA_NON_RETRY_BEFORE");
  const after = stringConstant("STABLE_METADATA_NON_RETRY_AFTER");

  assert.ok(Buffer.byteLength(after) <= Buffer.byteLength(before));
  assert.match(after, /you have not agreed to the xcode license agreements/);
  assert.match(after, /not a git repository/);
  assert.match(after, /\/i\.test/);
  assert.match(patchScript, /replaceStableMetadataNonRetryGuard/);
  assert.match(patchScript, /stable-metadata-git-retry/);
  assert.match(patchScript, /if\s*\(\s*!sawStableMetadataRetry\s*\|\|/);
  assert.doesNotMatch(patchScript, /uV=0/);

  const permanentError = new Function("e", after);
  assert.equal(permanentError(new Error("You have not agreed to the Xcode license agreements")), true);
  assert.equal(permanentError(new Error("fatal: not a git repository")), true);
  assert.equal(permanentError("NOT A GIT REPOSITORY"), true);
  assert.equal(permanentError(new Error("temporary git index lock")), false);
});

test("the split Ultra picker preimage is recognized and widened", () => {
  assert.match(patchScript, /LEGACY_SPLIT_POWER_SELECTIONS/);
  assert.match(patchScript, /LEGACY_SPLIT_POWER_SELECTION_FILTER/);
  assert.match(patchScript, /CURRENT_SPLIT_POWER_SELECTION_FILTER/);
  assert.match(patchScript, /hasSplitUltraPicker/);
});

test("the composer trigger uses the same English reasoning vocabulary as the picker", () => {
  assert.match(patchScript, /replaceComposerCollapsedReasoningLabel/);
  assert.match(patchScript, /replaceComposerVisibleReasoningLabel/);
  assert.match(patchScript, /replaceReasoningMenuVisibleLabels/);
  assert.match(patchScript, /replaceReasoningLabelDefaults/);
  assert.match(patchScript, /\.defaultMessage/);
  assert.match(patchScript, /defaultMessage:`Low`/);
  assert.match(patchScript, /defaultMessage:`Extra high`/);
  assert.match(patchScript, /defaultMessage:`ULTRA`/);
});

test("Luna Ultra remains a visual alias for max reasoning", () => {
  assert.match(
    patchScript,
    /t===`luna`&&e===`ultra`\?`max`:e/,
  );
});

test("the model picker payload keeps the system cursor", () => {
  const css = zlib
    .gunzipSync(Buffer.from(stringConstant("CUSTOM_PICKER_CSS_GZIP_BASE64"), "base64"))
    .toString("utf8");

  assert.doesNotMatch(css, /cursor\s*:\s*crosshair/i);
});

test("ASAR entries start after aligned header pickle padding", () => {
  assert.match(patchScript, /const headerPickleSize = prefix\.readUInt32LE\(4\)/);
  assert.match(patchScript, /const filesOffset = 8 \+ headerPickleSize/);
  assert.doesNotMatch(patchScript, /filesOffset:\s*16 \+ headerJsonSize/);
});
