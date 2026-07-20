import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");

const EXACT_PATCHES = [
  {
    before: ",s=i&&e!==`amazonBedrock`;",
    after: ",s=0&&e!==`amazonBedrock`;",
  },
  {
    before: ",u=s&&e!==`amazonBedrock`,",
    after: ",u=0&&e!==`amazonBedrock`,",
  },
  {
    before: ',s=i&&e!=="amazonBedrock";',
    after: ',s=0&&e!=="amazonBedrock";',
  },
  {
    before: ',u=s&&e!=="amazonBedrock",',
    after: ',u=0&&e!=="amazonBedrock",',
  },
];

const PATCH_NEEDLES = [
  /([,;][A-Za-z_$][\w$]*=)([A-Za-z_$])(&&[A-Za-z_$][\w$]*!==`amazonBedrock`[,;])/g,
  /([,;][A-Za-z_$][\w$]*=)([A-Za-z_$])(&&[A-Za-z_$][\w$]*!=="amazonBedrock"[,;])/g,
];

const APPLIED_NEEDLES = [
  /[,;][A-Za-z_$][\w$]*=0&&[A-Za-z_$][\w$]*!==`amazonBedrock`[,;]/,
  /[,;][A-Za-z_$][\w$]*=0&&[A-Za-z_$][\w$]*!=="amazonBedrock"[,;]/,
];

const HISTORY_PROVIDER_FILTER_BEFORE = "modelProviders:null";
const HISTORY_PROVIDER_FILTER_AFTER = "modelProviders:[]  ";
const STABLE_METADATA_NON_RETRY_BEFORE =
  "return(e instanceof Error?e.message:String(e)).toLowerCase().includes(`you have not agreed to the xcode license agreements`)";
const STABLE_METADATA_NON_RETRY_AFTER =
  "return/you have not agreed to the xcode license agreements|not a git repository/i.test(e instanceof Error?e.message:e)";

const LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER = "/*codexdeck-sol-max*/";
const PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS = [
  "/*codexdeck-power-v2*/",
  "/*codexdeck-power-v3*/",
  "/*codexdeck-power-v4*/",
];
const POWER_SELECTIONS_PATCH_MARKER = "/*codexdeck-power-v5*/";
const TERRA_LOW_POWER_SELECTION =
  "{id:`gpt-5.6-terra:low`,model:`gpt-5.6-terra`,modelLabel:`5.6 Terra`,reasoningEffort:`low`}";
const LEGACY_SOL_POWER_SELECTIONS = [
  TERRA_LOW_POWER_SELECTION,
  "{id:`gpt-5.6-sol:low`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`low`}",
  "{id:`gpt-5.6-sol:medium`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`medium`}",
  "{id:`gpt-5.6-sol:high`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`high`}",
  "{id:`gpt-5.6-sol:xhigh`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`xhigh`}",
  "{id:`gpt-5.6-sol:ultra`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`ultra`}",
].join(",");
const LEGACY_SPLIT_POWER_SELECTIONS = [
  TERRA_LOW_POWER_SELECTION,
  "{id:`gpt-5.6-sol:low`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`low`}",
  "{id:`gpt-5.6-sol:medium`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`medium`}",
  "{id:`gpt-5.6-sol:high`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`high`}",
  "{id:`gpt-5.6-sol:xhigh`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`xhigh`}",
].join(",");
const MANAGED_POWER_SELECTIONS_EXPRESSION =
  "`terra,sol,luna`.split`,`" +
  ".flatMap(t=>`low,medium,high,xhigh,max,ultra`.split`,`" +
  ".map(e=>({id:`gpt-5.6-${t}:${e}`,model:`gpt-5.6-${t}`," +
  "modelLabel:`5.6 ${t[0].toUpperCase()+t.slice(1)}`," +
  "reasoningEffort:t===`luna`&&e===`ultra`?`max`:e})))";
const LEGACY_POWER_SELECTION_FILTER =
  "function K(e){return q.flatMap((t,n)=>e?.some(e=>e.model===t.model&&e.supportedReasoningEfforts.some(({reasoningEffort:e})=>e===t.reasoningEffort))?[{...t,powerSettingIndex:n}]:[])}";
const LEGACY_FALLBACK_POWER_SELECTION_FILTER =
  "function K(e){let t=q(ce,e);if(t.length>=4)return t;let n=q(le,e);return n.length>=4?n:[]}";
const LEGACY_SPLIT_POWER_SELECTION_FILTER =
  "function ae(e,t=!1){let n=K(t?[...q,le]:q,e);if(n.length>=4)return n;let r=K(ue,e);return r.length>=4?r:[]}";
const CURRENT_LEGACY_POWER_SELECTION_FILTER =
  "function K(e){return q.filter(t=>t.modelLabel=e?.find(e=>e.model==t.model)?.displayName)}";
const CURRENT_FALLBACK_POWER_SELECTION_FILTER =
  "function K(e){return ce.filter(t=>t.modelLabel=e?.find(e=>e.model==t.model)?.displayName)}";
const CURRENT_SPLIT_POWER_SELECTION_FILTER =
  "function ae(e){return q.filter(t=>t.modelLabel=e?.find(e=>e.model==t.model)?.displayName)}";
const PREVIOUS_V5_LEGACY_POWER_SELECTION_FILTER =
  "function K(e){return q.filter(t=>e?.some(e=>e.model===t.model))}";
const PREVIOUS_V5_FALLBACK_POWER_SELECTION_FILTER =
  "function K(e){return ce.filter(t=>e?.some(e=>e.model===t.model))}";
const PREVIOUS_CURRENT_POWER_SELECTION_FILTER =
  "function K(e,t){return oe(e?.filter(e=>e.model===t))}";
const POWER_SELECTIONS_EXPORT_NAME = "h";
const COLLAPSED_REASONING_LABELS_JSON =
  "{\"low\":\"Low\",\"medium\":\"Medium\",\"high\":\"High\",\"xhigh\":\"Extra high\",\"max\":\"Max\",\"ultra\":\"ULTRA\"}";
const ZH_CN_MAX_LABEL_BEFORE = "\"composer.mode.local.reasoning.max.label\":`最高`";
const ZH_CN_MAX_LABEL_AFTER = "\"composer.mode.local.reasoning.max.label\":`Max`";
const ZH_CN_ULTRA_LABEL_BEFORE = '"composer.mode.local.reasoning.ultra.label":`极高`';
const ZH_CN_ULTRA_LABEL_AFTER = '"composer.mode.local.reasoning.ultra.label":`超高`';
const REASONING_LABEL_DEFAULT_REPLACEMENTS = [
  ["defaultMessage:`Light`", "defaultMessage:`Low`"],
  ["defaultMessage:`Extra High`", "defaultMessage:`Extra high`"],
  ["defaultMessage:`Ultra`", "defaultMessage:`ULTRA`"],
];

const PATCH_VERSION = process.env.CODEXDECK_PATCH_VERSION?.trim() || "model-picker-v23";
const sourceCodexVersion = process.env.CODEXDECK_SOURCE_CODEX_VERSION?.trim() || null;
const sourceAsarHashFromEnv = process.env.CODEXDECK_SOURCE_ASAR_HASH?.trim() || null;
const CUSTOM_PICKER_UI_MARKER = "codexdeck-model-picker-ui-v7";
const CUSTOM_PICKER_CSS_MARKER = ".cdpv8";
const CUSTOM_PICKER_COMPONENT_SHA256 =
  "4172a556d950840debd0b40f0c652a59e030fb7720f76d0d6d8c3392f4d90faa";
const CUSTOM_PICKER_CSS_SHA256 =
  "d8a366bf2ebb7cfeccf479de46cacb5ccedce36324c6e95552dc2ad7837bc2dc";
const ORIGINAL_POWER_SLIDER_FUNCTION_SHA256 =
  "7a560ef9b0622ba49133a9f354787cef3eea2b773839b4122cf7dc67c0886bd7";
const ORIGINAL_POWER_SLIDER_CSS_SHA256 =
  "8471822c996dbfa4c600439679d99a77ab27ee37e9aee0fc413c326ab1c18fdb";
const CUSTOM_PICKER_COMPONENT_GZIP_BASE64 =
  "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";
const CUSTOM_PICKER_CSS_GZIP_BASE64 =
  "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";

function decodeEmbeddedPayload(base64, expectedHash, label) {
  const bytes = zlib.gunzipSync(Buffer.from(base64, "base64"));
  if (sha256(bytes) !== expectedHash) {
    throw new Error(`Embedded ${label} payload failed its SHA256 check.`);
  }
  return bytes.toString("utf8");
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function countOccurrences(text, needle) {
  return text.split(needle).length - 1;
}

function regexMatches(text, pattern) {
  const flags = pattern.flags.includes("g") ? pattern.flags : `${pattern.flags}g`;
  return [...text.matchAll(new RegExp(pattern.source, flags))];
}

function readAsarHeader(asarPath) {
  const fd = fs.openSync(asarPath, "r");
  try {
    const prefix = Buffer.alloc(16);
    fs.readSync(fd, prefix, 0, prefix.length, 0);
    const headerPickleSize = prefix.readUInt32LE(4);
    const headerJsonSize = prefix.readUInt32LE(12);
    const headerEnd = 16 + headerJsonSize;
    const filesOffset = 8 + headerPickleSize;
    const paddingSize = filesOffset - headerEnd;
    if (paddingSize < 0 || paddingSize > 3 || filesOffset > fs.fstatSync(fd).size) {
      throw new Error(`Invalid ASAR header layout: ${asarPath}`);
    }
    const headerBytes = Buffer.alloc(headerJsonSize);
    fs.readSync(fd, headerBytes, 0, headerJsonSize, 16);
    if (paddingSize > 0) {
      const padding = Buffer.alloc(paddingSize);
      fs.readSync(fd, padding, 0, paddingSize, headerEnd);
      if (padding.some((byte) => byte !== 0)) {
        throw new Error(`Invalid ASAR header padding: ${asarPath}`);
      }
    }
    return {
      headerJsonSize,
      headerBytes,
      header: JSON.parse(headerBytes.toString("utf8")),
      filesOffset,
    };
  } finally {
    fs.closeSync(fd);
  }
}

function listAsarFiles(header) {
  const out = [];
  function walk(node, parts) {
    if (!node || typeof node !== "object") return;
    if (node.files && typeof node.files === "object") {
      for (const [name, child] of Object.entries(node.files)) {
        walk(child, [...parts, name]);
      }
      return;
    }
    if (typeof node.offset === "string" && typeof node.size === "number") {
      out.push({ path: parts.join("/"), entry: node });
    }
  }
  walk(header, []);
  return out;
}

function readAsarEntry(asarPath, filesOffset, entry) {
  const fd = fs.openSync(asarPath, "r");
  try {
    const offset = filesOffset + Number(entry.offset);
    const bytes = Buffer.alloc(entry.size);
    fs.readSync(fd, bytes, 0, bytes.length, offset);
    return { offset, bytes };
  } finally {
    fs.closeSync(fd);
  }
}

function replaceModelPickerGate(text) {
  const appliedMatches = APPLIED_NEEDLES.flatMap((needle) => regexMatches(text, needle));
  const pendingMatches = PATCH_NEEDLES.flatMap((needle) => regexMatches(text, needle));
  if (
    appliedMatches.length > 1 ||
    pendingMatches.length > 1 ||
    (appliedMatches.length === 1 && pendingMatches.length === 1)
  ) {
    return { state: "ambiguous" };
  }
  if (appliedMatches.length === 1) {
    return { state: "patched", text };
  }
  if (pendingMatches.length === 1) {
    const match = pendingMatches[0];
    const replacement = `${match[1]}0${match[3]}`;
    return {
      state: "patchable",
      text: `${text.slice(0, match.index)}${replacement}${text.slice(match.index + match[0].length)}`,
    };
  }

  const appliedExact = EXACT_PATCHES.filter(({ after }) => text.includes(after));
  const pendingExact = EXACT_PATCHES.flatMap((exact) =>
    Array.from({ length: countOccurrences(text, exact.before) }, () => exact),
  );
  if (
    appliedExact.length > 1 ||
    pendingExact.length > 1 ||
    (appliedExact.length === 1 && pendingExact.length === 1)
  ) {
    return { state: "ambiguous" };
  }
  if (appliedExact.length === 1) {
    return { state: "patched", text };
  }
  if (pendingExact.length === 1) {
    const exact = pendingExact[0];
    return { state: "patchable", text: text.replace(exact.before, exact.after) };
  }

  return { state: "missing" };
}

const MAX_REASONING_EFFORT_FILTER_BEFORE =
  /\(\{reasoningEffort:([A-Za-z_$][\w$]*)\}\)=>([A-Za-z_$][\w$]*)\(\1\)&&([A-Za-z_$][\w$]*)\.has\(\1\)/g;
const MAX_REASONING_EFFORT_FILTER_AFTER =
  /\(\{reasoningEffort:([A-Za-z_$][\w$]*)\}\)=>([A-Za-z_$][\w$]*)\(\1\)&&\(\1===`max`\|\|([A-Za-z_$][\w$]*)\.has\(\1\)\)/;

function isCompactedModelPickerGate(text) {
  return /,[A-Za-z_$][\w$]*=0,[A-Za-z_$][\w$]*=[A-Za-z_$][\w$]*\.some\(/.test(text);
}

function compactModelPickerGate(text) {
  const matches = [
    ...text.matchAll(
      /([,;])([A-Za-z_$][\w$]*)=0&&[A-Za-z_$][\w$]*!==(?:`amazonBedrock`|"amazonBedrock")([,;])/g,
    ),
  ];
  if (matches.length === 0) {
    return { state: "missing" };
  }
  if (matches.length > 1) {
    return { state: "ambiguous" };
  }

  const match = matches[0];
  const replacement = `${match[1]}${match[2]}=0${match[3]}`;
  return {
    state: "patchable",
    text: `${text.slice(0, match.index)}${replacement}${text.slice(match.index + match[0].length)}`,
  };
}

function replaceMaxReasoningEffortFilter(text) {
  const appliedMatches = regexMatches(text, MAX_REASONING_EFFORT_FILTER_AFTER);
  const pendingMatches = regexMatches(text, MAX_REASONING_EFFORT_FILTER_BEFORE);
  if (
    appliedMatches.length > 1 ||
    pendingMatches.length > 1 ||
    (appliedMatches.length === 1 && pendingMatches.length === 1)
  ) {
    return { state: "ambiguous" };
  }
  if (appliedMatches.length === 1) {
    return { state: "patched", text };
  }
  if (pendingMatches.length === 0) {
    return { state: "missing" };
  }

  const match = pendingMatches[0];
  const replacement =
    `({reasoningEffort:${match[1]}})=>${match[2]}(${match[1]})&&` +
    `(${match[1]}===\`max\`||${match[3]}.has(${match[1]}))`;
  return {
    state: "patchable",
    text: `${text.slice(0, match.index)}${replacement}${text.slice(match.index + match[0].length)}`,
  };
}

function padToByteLength(text, expectedLength) {
  const currentLength = Buffer.byteLength(text, "utf8");
  if (currentLength > expectedLength) {
    return null;
  }
  return `${text}${" ".repeat(expectedLength - currentLength)}`;
}

function replaceModelListFilter(text) {
  const originalLength = Buffer.byteLength(text, "utf8");
  const gate = replaceModelPickerGate(text);
  const maxFilter = replaceMaxReasoningEffortFilter(gate.text ?? text);

  if (gate.state === "missing" && isCompactedModelPickerGate(text)) {
    if (maxFilter.state === "patched") {
      return { state: "patched", text };
    }
    return { state: "unsafe-size-change" };
  }
  if (["ambiguous", "missing", "unsafe-size-change"].includes(gate.state)) {
    return { state: gate.state };
  }
  if (["ambiguous", "missing", "unsafe-size-change"].includes(maxFilter.state)) {
    return { state: maxFilter.state };
  }
  if (gate.state === "patched" && maxFilter.state === "patched") {
    return { state: "patched", text };
  }
  if (maxFilter.state === "patched") {
    return { state: "patchable", text: gate.text };
  }

  const compactGate = compactModelPickerGate(maxFilter.text);
  if (compactGate.state !== "patchable") {
    return compactGate;
  }
  const patched = padToByteLength(compactGate.text, originalLength);
  if (!patched) {
    return { state: "unsafe-size-change" };
  }
  return { state: "patchable", text: patched };
}

function containsPowerPickerSignature(text) {
  const hasSplitUltraPicker =
    text.includes(LEGACY_SPLIT_POWER_SELECTIONS) &&
    text.includes(LEGACY_SPLIT_POWER_SELECTION_FILTER);
  return (
    text.includes("gpt-5.6-terra:low") &&
    text.includes("gpt-5.6-sol:xhigh") &&
    text.includes("gpt-5.6-sol:ultra") &&
    (text.includes("gpt-5.6-luna") ||
      text.includes(LEGACY_SOL_POWER_SELECTIONS) ||
      hasSplitUltraPicker)
  );
}

function padSameSizeReplacement(before, after) {
  const beforeLength = Buffer.byteLength(before, "utf8");
  const afterLength = Buffer.byteLength(after, "utf8");
  if (afterLength > beforeLength) {
    return null;
  }
  return `${after}${" ".repeat(beforeLength - afterLength)}`;
}

function isStableMetadataRetryAsset(text) {
  return text.includes("stable-metadata") && text.includes("Unknown method: process/spawn");
}

function replaceStableMetadataNonRetryGuard(text) {
  const beforeCount = countOccurrences(text, STABLE_METADATA_NON_RETRY_BEFORE);
  const afterCount = countOccurrences(text, STABLE_METADATA_NON_RETRY_AFTER);
  if (
    beforeCount > 1 ||
    afterCount > 1 ||
    (beforeCount === 1 && afterCount === 1)
  ) {
    return { state: "ambiguous" };
  }
  if (beforeCount === 0 && afterCount === 0) {
    return { state: "missing" };
  }

  let patchedText = text;
  let changed = false;
  if (beforeCount === 1) {
    const replacement = padSameSizeReplacement(
      STABLE_METADATA_NON_RETRY_BEFORE,
      STABLE_METADATA_NON_RETRY_AFTER,
    );
    if (!replacement) return { state: "unsafe-size-change" };
    patchedText = patchedText.replace(STABLE_METADATA_NON_RETRY_BEFORE, replacement);
    changed = true;
  }
  if (patchedText.includes(HISTORY_PROVIDER_FILTER_BEFORE)) {
    patchedText = patchedText.replaceAll(
      HISTORY_PROVIDER_FILTER_BEFORE,
      HISTORY_PROVIDER_FILTER_AFTER,
    );
    changed = true;
  }

  return changed ? { state: "patchable", text: patchedText } : { state: "patched", text };
}

function findFunctionEnd(text, openBraceOffset) {
  let depth = 0;
  let quote = null;
  let escaped = false;
  let lineComment = false;
  let blockComment = false;
  for (let index = openBraceOffset; index < text.length; index += 1) {
    const char = text[index];
    const next = text[index + 1];
    if (lineComment) {
      if (char === "\n" || char === "\r") lineComment = false;
      continue;
    }
    if (blockComment) {
      if (char === "*" && next === "/") {
        blockComment = false;
        index += 1;
      }
      continue;
    }
    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === quote) {
        quote = null;
      }
      continue;
    }
    if (char === "/" && next === "/") {
      lineComment = true;
      index += 1;
      continue;
    }
    if (char === "/" && next === "*") {
      blockComment = true;
      index += 1;
      continue;
    }
    if (char === '"' || char === "'" || char === "`") {
      quote = char;
      continue;
    }
    if (char === "{") depth += 1;
    if (char === "}") {
      depth -= 1;
      if (depth === 0) return index + 1;
    }
  }
  return -1;
}

function findPowerSliderFunction(text) {
  const exports = [
    ...text.matchAll(/export\{([A-Za-z_$][\w$]*) as ModelPickerPowerSliderImpl\}/g),
  ];
  if (exports.length === 0) return { state: "missing" };
  if (exports.length > 1) return { state: "ambiguous" };

  const name = exports[0][1];
  const signature = `function ${name}(`;
  const starts = [];
  let cursor = 0;
  while ((cursor = text.indexOf(signature, cursor)) >= 0) {
    starts.push(cursor);
    cursor += signature.length;
  }
  if (starts.length === 0) return { state: "missing" };
  if (starts.length > 1) return { state: "ambiguous" };

  const start = starts[0];
  const openBrace = text.indexOf("{", start + signature.length);
  const end = openBrace < 0 ? -1 : findFunctionEnd(text, openBrace);
  if (end < 0) return { state: "missing" };
  const source = text.slice(start, end);
  const reactAliases = [
    ...new Set([...source.matchAll(/\b([A-Za-z_$][\w$]*)\.useState\b/g)].map((match) => match[1])),
  ];
  if (reactAliases.length !== 1) {
    return { state: reactAliases.length > 1 ? "ambiguous" : "missing" };
  }
  return { state: "found", name, reactAlias: reactAliases[0], source, start, end };
}

let customPickerComponentSource;
let customPickerCssSource;

function embeddedCustomPickerComponent() {
  customPickerComponentSource ??= decodeEmbeddedPayload(
    CUSTOM_PICKER_COMPONENT_GZIP_BASE64,
    CUSTOM_PICKER_COMPONENT_SHA256,
    "model picker component",
  );
  return customPickerComponentSource;
}

function embeddedCustomPickerCss() {
  customPickerCssSource ??= decodeEmbeddedPayload(
    CUSTOM_PICKER_CSS_GZIP_BASE64,
    CUSTOM_PICKER_CSS_SHA256,
    "model picker CSS",
  );
  return customPickerCssSource;
}

function replacePowerSliderComponent(text) {
  const markerCount = countOccurrences(text, CUSTOM_PICKER_UI_MARKER);
  if (markerCount > 1) return { state: "ambiguous" };
  const target = findPowerSliderFunction(text);
  if (target.state !== "found") return target;
  if (markerCount === 1) {
    const reactBinding = target.source.match(
      new RegExp(
        `"${CUSTOM_PICKER_UI_MARKER}";const ${escapeRegExp(target.reactAlias)}=([A-Za-z_$][\\w$]*),`,
      ),
    );
    if (!reactBinding) return { state: "unsupported-preimage" };
    const component = embeddedCustomPickerComponent()
      .replace("__COMPONENT__", target.name)
      .replaceAll("__REACT__", reactBinding[1])
      .replace(/\r?\n$/, "");
    return target.source === component
      ? { state: "patched", text }
      : { state: "unsupported-preimage" };
  }
  if (sha256(Buffer.from(target.source, "utf8")) !== ORIGINAL_POWER_SLIDER_FUNCTION_SHA256) {
    return { state: "unsupported-preimage" };
  }

  const component = embeddedCustomPickerComponent()
    .replace("__COMPONENT__", target.name)
    .replaceAll("__REACT__", target.reactAlias);
  const replacement = padSameSizeReplacement(target.source, component);
  if (!replacement) return { state: "unsafe-size-change" };
  return {
    state: "patchable",
    text: `${text.slice(0, target.start)}${replacement}${text.slice(target.end)}`,
  };
}

function replacePowerSliderCss(text) {
  const markerCount = countOccurrences(text, CUSTOM_PICKER_CSS_MARKER);
  if (markerCount > 1) return { state: "ambiguous" };
  if (markerCount === 1) {
    const css = embeddedCustomPickerCss();
    return text.startsWith(css) && text.slice(css.length).trim() === ""
      ? { state: "patched", text }
      : { state: "unsupported-preimage" };
  }
  if (sha256(Buffer.from(text, "utf8")) !== ORIGINAL_POWER_SLIDER_CSS_SHA256) {
    return { state: "unsupported-preimage" };
  }
  const replacement = padSameSizeReplacement(text, embeddedCustomPickerCss());
  if (!replacement) return { state: "unsafe-size-change" };
  return { state: "patchable", text: replacement };
}

function isPowerSliderJsEntry(entryPath) {
  return /^webview\/assets\/model-picker-power-slider-impl-.*\.js$/.test(entryPath);
}

function isPowerSliderCssEntry(entryPath) {
  return /^webview\/assets\/model-picker-power-slider-impl-.*\.css$/.test(entryPath);
}

function powerSelectionsReplacement(assignment) {
  return `${assignment}=[${POWER_SELECTIONS_PATCH_MARKER}...${MANAGED_POWER_SELECTIONS_EXPRESSION}]`;
}

const COLLAPSED_POWER_LABEL_BEFORE =
  /function ([A-Za-z_$][\w$]*)\(([A-Za-z_$][\w$]*),([A-Za-z_$][\w$]*)\)\{return`\$\{\2\.modelLabel\} \$\{\3\.formatMessage\(([A-Za-z_$][\w$]*)\[\2\.reasoningEffort\]\)\}`\}/g;
const COLLAPSED_POWER_LABEL_AFTER =
  /function ([A-Za-z_$][\w$]*)\(([A-Za-z_$][\w$]*),([A-Za-z_$][\w$]*)\)\{return`\$\{\2\.modelLabel\} \$\{([A-Za-z_$][\w$]*)\[\2\.reasoningEffort\]\}`\}/g;

function replaceCollapsedPowerSelectionLabel(text) {
  const beforeMatches = regexMatches(text, COLLAPSED_POWER_LABEL_BEFORE);
  const afterMatches = regexMatches(text, COLLAPSED_POWER_LABEL_AFTER);
  if (
    beforeMatches.length > 1 ||
    afterMatches.length > 1 ||
    (beforeMatches.length === 1 && afterMatches.length === 1)
  ) {
    return { state: "ambiguous" };
  }

  if (afterMatches.length === 1) {
    const labelsVariable = afterMatches[0][4];
    const assignment = `${labelsVariable}=${COLLAPSED_REASONING_LABELS_JSON}`;
    const count = countOccurrences(text, assignment);
    if (count > 1) return { state: "ambiguous" };
    return count === 1 ? { state: "patched", text } : { state: "missing" };
  }
  if (beforeMatches.length === 0) return { state: "missing" };

  const formatterMatch = beforeMatches[0];
  const labelsVariable = formatterMatch[4];
  const assignmentPrefix = `${labelsVariable}=`;
  const assignmentCandidates = [];
  let cursor = formatterMatch.index + formatterMatch[0].length;
  while ((cursor = text.indexOf(assignmentPrefix, cursor)) >= 0) {
    const objectOffset = cursor + assignmentPrefix.length;
    const objectEnd = text[objectOffset] === "{" ? findFunctionEnd(text, objectOffset) : -1;
    if (objectEnd >= 0) {
      const source = text.slice(objectOffset, objectEnd);
      if (
        source.includes("composer.modelPicker.work.reasoning.standard.label") &&
        source.includes("composer.modelPicker.work.reasoning.extended.label")
      ) {
        assignmentCandidates.push({ objectOffset, objectEnd, source });
      }
    }
    cursor += assignmentPrefix.length;
  }
  if (assignmentCandidates.length !== 1) {
    return { state: assignmentCandidates.length > 1 ? "ambiguous" : "missing" };
  }

  const { objectOffset, objectEnd, source: originalLabels } = assignmentCandidates[0];

  const formatter = `function ${formatterMatch[1]}(${formatterMatch[2]},${formatterMatch[3]}){return\`\${${formatterMatch[2]}.modelLabel} \${${labelsVariable}[${formatterMatch[2]}.reasoningEffort]}\`}`;
  const paddedFormatter = padSameSizeReplacement(formatterMatch[0], formatter);
  const paddedLabels = padSameSizeReplacement(originalLabels, COLLAPSED_REASONING_LABELS_JSON);
  if (!paddedFormatter || !paddedLabels) return { state: "unsafe-size-change" };

  let patched =
    text.slice(0, formatterMatch.index) +
    paddedFormatter +
    text.slice(formatterMatch.index + formatterMatch[0].length);
  patched = patched.slice(0, objectOffset) + paddedLabels + patched.slice(objectEnd);
  return { state: "patchable", text: patched };
}

const REASONING_MENU_LABEL_ANCHOR =
  /([A-Za-z_$][\w$]*)=([A-Za-z_$][\w$]*)\[([A-Za-z_$][\w$]*)\],([A-Za-z_$][\w$]*);/g;

function analyzeReasoningMenuVisibleLabelCandidate(text, anchor) {
  const descriptor = anchor[1];
  const labels = anchor[2];
  const rendered = anchor[4];
  const windowStart = anchor.index + anchor[0].length;
  const windowText = text.slice(windowStart, Math.min(text.length, windowStart + 512));
  const selectedBefore = regexMatches(
    windowText,
    new RegExp(
      `${escapeRegExp(rendered)}=\\(0,[A-Za-z_$][\\w$]*\\.jsx\\)\\([A-Za-z_$][\\w$]*,\\{\\.\\.\\.${escapeRegExp(descriptor)}\\}\\)`,
      "g",
    ),
  );
  const selectedAfter = regexMatches(
    windowText,
    new RegExp(`${escapeRegExp(rendered)}=${escapeRegExp(descriptor)}\\.defaultMessage`, "g"),
  );
  const menuBefore = regexMatches(
    text,
    new RegExp(
      `children:\\(0,[A-Za-z_$][\\w$]*\\.jsx\\)\\([A-Za-z_$][\\w$]*,\\{\\.\\.\\.${escapeRegExp(labels)}\\[([A-Za-z_$][\\w$]*)\\]\\}\\)`,
      "g",
    ),
  );
  const menuAfter = regexMatches(
    text,
    new RegExp(
      `children:${escapeRegExp(labels)}\\[([A-Za-z_$][\\w$]*)\\]\\.defaultMessage`,
      "g",
    ),
  );
  return {
    descriptor,
    labels,
    rendered,
    windowStart,
    selectedBefore,
    selectedAfter,
    menuBefore,
    menuAfter,
  };
}

function replaceReasoningMenuVisibleLabels(text) {
  const candidates = regexMatches(text, REASONING_MENU_LABEL_ANCHOR)
    .map((anchor) => analyzeReasoningMenuVisibleLabelCandidate(text, anchor))
    .filter(
      (candidate) =>
        candidate.selectedBefore.length + candidate.selectedAfter.length === 1 &&
        candidate.menuBefore.length + candidate.menuAfter.length === 2,
    );
  if (candidates.length === 0) return { state: "missing" };
  if (candidates.length > 1) return { state: "ambiguous" };

  const candidate = candidates[0];
  if (candidate.menuBefore.length > 0 && candidate.menuAfter.length > 0) {
    return { state: "ambiguous" };
  }
  if (candidate.selectedAfter.length === 1 && candidate.menuAfter.length === 2) {
    return { state: "patched", text };
  }

  const replacements = [];
  if (candidate.selectedBefore.length === 1) {
    const match = candidate.selectedBefore[0];
    const replacement = `${candidate.rendered}=${candidate.descriptor}.defaultMessage`;
    const padded = padSameSizeReplacement(match[0], replacement);
    if (!padded) return { state: "unsafe-size-change" };
    replacements.push({
      start: candidate.windowStart + match.index,
      end: candidate.windowStart + match.index + match[0].length,
      replacement: padded,
    });
  }
  for (const match of candidate.menuBefore) {
    const replacement = `children:${candidate.labels}[${match[1]}].defaultMessage`;
    const padded = padSameSizeReplacement(match[0], replacement);
    if (!padded) return { state: "unsafe-size-change" };
    replacements.push({
      start: match.index,
      end: match.index + match[0].length,
      replacement: padded,
    });
  }

  let patched = text;
  for (const replacement of replacements.sort((left, right) => right.start - left.start)) {
    patched =
      patched.slice(0, replacement.start) +
      replacement.replacement +
      patched.slice(replacement.end);
  }
  return { state: "patchable", text: patched };
}

function hasCompletePowerSelections(text) {
  const currentFilterCount = [
    CURRENT_LEGACY_POWER_SELECTION_FILTER,
    CURRENT_FALLBACK_POWER_SELECTION_FILTER,
    CURRENT_SPLIT_POWER_SELECTION_FILTER,
  ].reduce((count, filter) => count + countOccurrences(text, filter), 0);
  return (
    validateCurrentPowerSelections(text).state === "patched" &&
    text.includes("terra,sol,luna") &&
    text.includes("gpt-5.6-${t}:${e}") &&
    text.includes("t===`luna`&&e===`ultra`?`max`:e") &&
    text.includes("low,medium,high,xhigh,max,ultra") &&
    currentFilterCount === 1 &&
    replaceCollapsedPowerSelectionLabel(text).state === "patched" &&
    replaceReasoningMenuVisibleLabels(text).state === "patched" &&
    countOccurrences(text, "`cdpw`") === 1
  );
}

function replacePowerSelectionFilter(text) {
  const currentFilters = [
    CURRENT_LEGACY_POWER_SELECTION_FILTER,
    CURRENT_FALLBACK_POWER_SELECTION_FILTER,
    CURRENT_SPLIT_POWER_SELECTION_FILTER,
  ].flatMap((candidate) =>
    Array.from({ length: countOccurrences(text, candidate) }, () => candidate),
  );
  if (currentFilters.length > 1) {
    return { state: "ambiguous" };
  }
  if (currentFilters.length === 1) {
    return { state: "patched", text };
  }

  const previousCurrentMatches = regexMatches(
    text,
    new RegExp(`${escapeRegExp(PREVIOUS_CURRENT_POWER_SELECTION_FILTER)} *`, "g"),
  );
  if (previousCurrentMatches.length > 1) {
    return { state: "ambiguous" };
  }
  if (previousCurrentMatches.length === 1) {
    const before = previousCurrentMatches[0][0];
    const replacement = padSameSizeReplacement(before, CURRENT_LEGACY_POWER_SELECTION_FILTER);
    if (!replacement) {
      return { state: "unsafe-size-change" };
    }
    return {
      state: "patchable",
      text: `${text.slice(0, previousCurrentMatches[0].index)}${replacement}${text.slice(
        previousCurrentMatches[0].index + before.length,
      )}`,
    };
  }

  const previousV5Replacements = [
    {
      before: PREVIOUS_V5_LEGACY_POWER_SELECTION_FILTER,
      after: CURRENT_LEGACY_POWER_SELECTION_FILTER,
    },
    {
      before: PREVIOUS_V5_FALLBACK_POWER_SELECTION_FILTER,
      after: CURRENT_FALLBACK_POWER_SELECTION_FILTER,
    },
  ].flatMap((replacement) =>
    regexMatches(text, new RegExp(`${escapeRegExp(replacement.before)} *`, "g")).map(
      (match) => ({ match, after: replacement.after }),
    ),
  );
  if (previousV5Replacements.length > 1) {
    return { state: "ambiguous" };
  }
  if (previousV5Replacements.length === 1) {
    const { match, after } = previousV5Replacements[0];
    const replacement = padSameSizeReplacement(match[0], after);
    if (!replacement) return { state: "unsafe-size-change" };
    return {
      state: "patchable",
      text: `${text.slice(0, match.index)}${replacement}${text.slice(match.index + match[0].length)}`,
    };
  }

  const replacements = [
    {
      before: LEGACY_POWER_SELECTION_FILTER,
      after: CURRENT_LEGACY_POWER_SELECTION_FILTER,
    },
    {
      before: LEGACY_FALLBACK_POWER_SELECTION_FILTER,
      after: CURRENT_FALLBACK_POWER_SELECTION_FILTER,
    },
    {
      before: LEGACY_SPLIT_POWER_SELECTION_FILTER,
      after: CURRENT_SPLIT_POWER_SELECTION_FILTER,
    },
  ];
  const matchingReplacements = replacements.flatMap((replacement) =>
    Array.from({ length: countOccurrences(text, replacement.before) }, () => replacement),
  );
  if (matchingReplacements.length === 0) {
    return { state: "missing" };
  }
  if (matchingReplacements.length > 1) {
    return { state: "ambiguous" };
  }

  const { before, after } = matchingReplacements[0];
  const count = text.split(before).length - 1;
  if (count === 0) {
    return { state: "missing" };
  }
  if (count > 1) {
    return { state: "ambiguous" };
  }

  const replacement = padSameSizeReplacement(
    before,
    after,
  );
  if (!replacement) {
    return { state: "unsafe-size-change" };
  }
  return {
    state: "patchable",
    text: text.replace(before, replacement),
  };
}

function findMatchingArrayBracket(text, openOffset) {
  let depth = 0;
  let quote = null;
  let escaped = false;
  for (let index = openOffset; index < text.length; index += 1) {
    const char = text[index];
    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === quote) {
        quote = null;
      }
      continue;
    }
    if (char === '"' || char === "'" || char === "`") {
      quote = char;
      continue;
    }
    if (char === "[") depth += 1;
    if (char === "]") {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return -1;
}

function currentPowerSelectionsRegion(text) {
  const markerCount = countOccurrences(text, POWER_SELECTIONS_PATCH_MARKER);
  if (markerCount === 0) return { state: "missing" };
  if (markerCount > 1) return { state: "ambiguous" };

  const markerOffset = text.indexOf(POWER_SELECTIONS_PATCH_MARKER);
  const assignment = text
    .slice(0, markerOffset)
    .match(/([A-Za-z_$][\w$]*)=\[[ \t]*$/);
  if (!assignment) return { state: "missing" };

  const start = markerOffset - assignment[0].length;
  const arrayOpen = start + assignment[0].indexOf("[");
  const arrayClose = findMatchingArrayBracket(text, arrayOpen);
  if (arrayClose < 0) return { state: "missing" };
  return {
    state: "found",
    assignment: assignment[1],
    start,
    end: arrayClose + 1,
  };
}

function validateCurrentPowerSelections(text) {
  const region = currentPowerSelectionsRegion(text);
  if (region.state !== "found") return region;
  const expected = powerSelectionsReplacement(region.assignment);
  return text.slice(region.start, region.end) === expected
    ? { state: "patched", text }
    : { state: "unsupported-preimage" };
}

function replaceMarkedPowerSelections(text, marker) {
  const count = text.split(marker).length - 1;
  if (count === 0) {
    return { state: "missing" };
  }
  if (count > 1) {
    return { state: "ambiguous" };
  }
  const markerOffset = text.indexOf(marker);
  const assignment = text
    .slice(0, markerOffset)
    .match(/([A-Za-z_$][\w$]*)=\[[ \t]*$/);
  if (!assignment) {
    return { state: "missing" };
  }

  const beforeOffset = markerOffset - assignment[0].length;
  const arrayOpenOffset = beforeOffset + assignment[0].indexOf("[");
  const arrayCloseOffset = findMatchingArrayBracket(text, arrayOpenOffset);
  if (arrayCloseOffset < 0) {
    return { state: "missing" };
  }
  const beforeEndOffset = arrayCloseOffset + 1;
  const before = text.slice(beforeOffset, beforeEndOffset);
  const replacement = padSameSizeReplacement(before, powerSelectionsReplacement(assignment[1]));
  if (!replacement) {
    return { state: "unsafe-size-change" };
  }
  return {
    state: "patchable",
    text: `${text.slice(0, beforeOffset)}${replacement}${text.slice(beforeEndOffset)}`,
  };
}

function replacePowerPickerMenuWidth(text) {
  const markerCount = countOccurrences(text, "`cdpw`");
  if (markerCount > 1) return { state: "ambiguous" };
  if (markerCount === 1) {
    return { state: "patched", text };
  }
  const matches = [
    ...text.matchAll(/([A-Za-z_$][\w$]*)\(`w-56`,([A-Za-z_$][\w$]*)\)/g),
  ];
  if (matches.length === 0) return { state: "missing" };
  if (matches.length > 1) return { state: "ambiguous" };
  const match = matches[0];
  const replacement = `${match[1]}(\`cdpw\`,${match[2]})`;
  return {
    state: "patchable",
    text: `${text.slice(0, match.index)}${replacement}${text.slice(match.index + match[0].length)}`,
  };
}

function replacePowerSelections(text) {
  let selections = { state: "patched", text };
  if (text.includes(POWER_SELECTIONS_PATCH_MARKER)) {
    selections = validateCurrentPowerSelections(text);
  } else if (PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS.some((marker) => text.includes(marker))) {
    const markers = PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS.filter((marker) => text.includes(marker));
    if (markers.length !== 1) return { state: "ambiguous" };
    selections = replaceMarkedPowerSelections(text, markers[0]);
  } else if (text.includes(LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER)) {
    selections = replaceMarkedPowerSelections(text, LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER);
  } else {
    const unifiedCount = countOccurrences(text, LEGACY_SOL_POWER_SELECTIONS);
    const splitCount = unifiedCount === 0 ? countOccurrences(text, LEGACY_SPLIT_POWER_SELECTIONS) : 0;
    if (unifiedCount > 1 || splitCount > 1) {
      return { state: "ambiguous" };
    }
    if (unifiedCount === 0 && splitCount === 0) return { state: "missing" };

    const legacySelection = unifiedCount === 1 ? LEGACY_SOL_POWER_SELECTIONS : LEGACY_SPLIT_POWER_SELECTIONS;
    const selectionsOffset = text.indexOf(legacySelection);
    const assignment = text.slice(0, selectionsOffset).match(/([A-Za-z_$][\w$]*)=\[[ \t]*$/);
    if (!assignment) {
      return { state: "missing" };
    }

    const beforeOffset = selectionsOffset - assignment[0].length;
    const closingBracketOffset = selectionsOffset + legacySelection.length;
    if (text[closingBracketOffset] !== "]") {
      return { state: "missing" };
    }

    const before = text.slice(beforeOffset, closingBracketOffset + 1);
    const after = powerSelectionsReplacement(assignment[1]);
    const replacement = padSameSizeReplacement(before, after);
    if (!replacement) {
      return { state: "unsafe-size-change" };
    }
    selections = {
      state: "patchable",
      text: `${text.slice(0, beforeOffset)}${replacement}${text.slice(beforeOffset + before.length)}`,
    };
  }

  if (selections.state !== "patched" && selections.state !== "patchable") {
    return selections;
  }

  const filter = replacePowerSelectionFilter(selections.text);
  if (filter.state !== "patched" && filter.state !== "patchable") {
    return filter;
  }

  const menuWidth = replacePowerPickerMenuWidth(filter.text);
  if (menuWidth.state !== "patched" && menuWidth.state !== "patchable") {
    return menuWidth;
  }

  const collapsedLabel = replaceCollapsedPowerSelectionLabel(menuWidth.text);
  if (collapsedLabel.state !== "patched" && collapsedLabel.state !== "patchable") {
    return collapsedLabel;
  }

  const visibleMenuLabels = replaceReasoningMenuVisibleLabels(collapsedLabel.text);
  if (visibleMenuLabels.state !== "patched" && visibleMenuLabels.state !== "patchable") {
    return visibleMenuLabels;
  }

  const patched = visibleMenuLabels.text;
  if (!hasCompletePowerSelections(patched)) {
    return { state: "missing" };
  }
  return {
    state:
      selections.state === "patched" &&
      filter.state === "patched" &&
      menuWidth.state === "patched" &&
      collapsedLabel.state === "patched" &&
      visibleMenuLabels.state === "patched"
        ? "patched"
        : "patchable",
    text: patched,
  };
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function findPowerSelectionScopeCall(text) {
  if (!text.includes("powerSelections:")) {
    return null;
  }
  const importMatch = text.match(
    /import\{([^}]*)\}from"\.\/model-and-reasoning-dropdown-[^"]+\.js";/,
  );
  if (!importMatch) {
    return null;
  }
  const exportMatch = importMatch[1].match(
    new RegExp(`(?:^|,)\\s*${POWER_SELECTIONS_EXPORT_NAME}\\s+as\\s+([A-Za-z_$][\\w$]*)\\b`),
  );
  if (!exportMatch) {
    return null;
  }

  const alias = exportMatch[1];
  const calls = [
    ...text.matchAll(
      new RegExp(
        `([A-Za-z_$][\\w$]*)=${escapeRegExp(alias)}\\(([A-Za-z_$][\\w$]*)(?:,([A-Za-z_$][\\w$]*))?\\)`,
        "g",
      ),
    ),
  ];
  if (calls.length === 0) {
    return { state: "missing" };
  }
  if (calls.length > 1) {
    return { state: "ambiguous" };
  }

  const match = calls[0];
  return {
    alias,
    index: match.index,
    length: match[0].length,
    model: match[2],
    result: match[1],
    selectedModel: match[3] ?? null,
  };
}

function isPowerSelectionScopeCandidate(text) {
  return findPowerSelectionScopeCall(text) !== null;
}

function fitPowerScopePatchToOriginalByteLength(text, expectedLength) {
  let fitted = text;
  if (Buffer.byteLength(fitted, "utf8") > expectedLength) {
    fitted = fitted.replace(
      ";\n//# sourceMappingURL=",
      "//# sourceMappingURL=",
    );
  }
  return padToByteLength(fitted, expectedLength);
}

function replacePowerSelectionScopeCall(text) {
  const call = findPowerSelectionScopeCall(text);
  if (call === null) {
    return { state: "missing" };
  }
  if (call.state) {
    return call;
  }
  if (call.selectedModel) {
    return { state: "patched", text };
  }

  const context = text.slice(Math.max(0, call.index - 1024), call.index + call.length);
  const selectedFromNeighbor = context.match(
    new RegExp(
      `=[A-Za-z_$][\\w$]*\\(${escapeRegExp(call.model)},([A-Za-z_$][\\w$]*)\\),${escapeRegExp(call.result)}=${escapeRegExp(call.alias)}\\(${escapeRegExp(call.model)}\\)$`,
    ),
  )?.[1];
  const selectedFromModelSettings = [
    ...text.slice(Math.max(0, call.index - 4096), call.index).matchAll(
      /([A-Za-z_$][\w$]*)=[A-Za-z_$][\w$]*\.model/g,
    ),
  ].at(-1)?.[1];
  const selectedModel = selectedFromNeighbor ?? selectedFromModelSettings;
  if (!selectedModel) {
    return { state: "missing" };
  }

  const originalLength = Buffer.byteLength(text, "utf8");
  const replacement = `${call.result}=${call.alias}(${call.model},${selectedModel})`;
  const patched =
    text.slice(0, call.index) + replacement + text.slice(call.index + call.length);
  const fitted = fitPowerScopePatchToOriginalByteLength(patched, originalLength);
  if (!fitted) {
    return { state: "unsafe-size-change" };
  }
  return { state: "patchable", text: fitted };
}

function replaceExactSameSizeLabel(text, before, after) {
  const beforeCount = countOccurrences(text, before);
  const afterCount = countOccurrences(text, after);
  if (beforeCount > 0 && afterCount > 0) {
    return { state: "ambiguous" };
  }
  if (afterCount === 1) {
    return { state: "patched", text };
  }
  if (afterCount > 1 || beforeCount > 1) {
    return { state: "ambiguous" };
  }
  if (beforeCount === 0) {
    return { state: "missing" };
  }

  const replacement = padSameSizeReplacement(before, after);
  if (!replacement) return { state: "unsafe-size-change" };
  return { state: "patchable", text: text.replace(before, replacement) };
}

function replaceZhCnReasoningLabels(entryPath, text) {
  if (!/^webview\/assets\/zh-CN-.*\.js$/.test(entryPath)) {
    return { state: "missing" };
  }

  let patchedText = text;
  let changed = false;
  for (const [before, after] of [
    [ZH_CN_MAX_LABEL_BEFORE, ZH_CN_MAX_LABEL_AFTER],
    [ZH_CN_ULTRA_LABEL_BEFORE, ZH_CN_ULTRA_LABEL_AFTER],
  ]) {
    const result = replaceExactSameSizeLabel(patchedText, before, after);
    if (result.state === "patchable") {
      patchedText = result.text;
      changed = true;
      continue;
    }
    if (result.state !== "patched") return result;
  }
  return changed ? { state: "patchable", text: patchedText } : { state: "patched", text };
}

const COMPOSER_COLLAPSED_REASONING_LABEL_BEFORE =
  /([A-Za-z_$][\w$]*)=([A-Za-z_$][\w$]*)=>\(\{\.\.\.\2,effortLabel:([A-Za-z_$][\w$]*)\.formatMessage\(([A-Za-z_$][\w$]*)\[\2\.reasoningEffort\]\)\}\)/g;
const COMPOSER_COLLAPSED_REASONING_LABEL_AFTER =
  /([A-Za-z_$][\w$]*)=([A-Za-z_$][\w$]*)=>\(\{\.\.\.\2,effortLabel:([A-Za-z_$][\w$]*)\[\2\.reasoningEffort\]\.defaultMessage\}\)/g;

function isComposerCollapsedReasoningLabelCandidate(entryPath, text) {
  return (
    entryPath.startsWith("webview/assets/") &&
    entryPath.endsWith(".js") &&
    text.includes("expandedContentWidth") &&
    text.includes("labelCandidates") &&
    text.includes("WorkTriggerEffortLabel")
  );
}

function replaceComposerCollapsedReasoningLabel(text) {
  const beforeMatches = regexMatches(text, COMPOSER_COLLAPSED_REASONING_LABEL_BEFORE);
  const afterMatches = regexMatches(text, COMPOSER_COLLAPSED_REASONING_LABEL_AFTER);
  if (
    beforeMatches.length > 1 ||
    afterMatches.length > 1 ||
    (beforeMatches.length === 1 && afterMatches.length === 1)
  ) {
    return { state: "ambiguous" };
  }
  if (afterMatches.length === 1) return { state: "patched", text };
  if (beforeMatches.length === 0) return { state: "missing" };

  const match = beforeMatches[0];
  const replacement = `${match[1]}=${match[2]}=>({...${match[2]},effortLabel:${match[4]}[${match[2]}.reasoningEffort].defaultMessage})`;
  const padded = padSameSizeReplacement(match[0], replacement);
  if (!padded) return { state: "unsafe-size-change" };
  return {
    state: "patchable",
    text: text.slice(0, match.index) + padded + text.slice(match.index + match[0].length),
  };
}

function replaceComposerVisibleReasoningLabel(text) {
  const anchorMatches = regexMatches(
    text,
    /let ([A-Za-z_$][\w$]*)=([A-Za-z_$][\w$]*)===`ultra`,([A-Za-z_$][\w$]*)=([A-Za-z_$][\w$]*)\[([A-Za-z_$][\w$]*)\],([A-Za-z_$][\w$]*);/g,
  ).filter((match) => match[2] === match[5]);
  if (anchorMatches.length === 0) return { state: "missing" };
  if (anchorMatches.length > 1) return { state: "ambiguous" };

  const anchor = anchorMatches[0];
  const descriptor = anchor[3];
  const rendered = anchor[6];
  const windowStart = anchor.index + anchor[0].length;
  const windowEnd = Math.min(text.length, windowStart + 512);
  const windowText = text.slice(windowStart, windowEnd);
  const beforeMatches = regexMatches(
    windowText,
    new RegExp(
      `${escapeRegExp(rendered)}=\\(0,[A-Za-z_$][\\w$]*\\.jsx\\)\\([A-Za-z_$][\\w$]*,\\{\\.\\.\\.${escapeRegExp(descriptor)}\\}\\)`,
      "g",
    ),
  );
  const afterMatches = regexMatches(
    windowText,
    new RegExp(`${escapeRegExp(rendered)}=${escapeRegExp(descriptor)}\\.defaultMessage`, "g"),
  );
  if (
    beforeMatches.length > 1 ||
    afterMatches.length > 1 ||
    (beforeMatches.length === 1 && afterMatches.length === 1)
  ) {
    return { state: "ambiguous" };
  }
  if (afterMatches.length === 1) return { state: "patched", text };
  if (beforeMatches.length === 0) return { state: "missing" };

  const match = beforeMatches[0];
  const replacement = `${rendered}=${descriptor}.defaultMessage`;
  const padded = padSameSizeReplacement(match[0], replacement);
  if (!padded) return { state: "unsafe-size-change" };
  const matchIndex = windowStart + match.index;
  return {
    state: "patchable",
    text: text.slice(0, matchIndex) + padded + text.slice(matchIndex + match[0].length),
  };
}

function replaceComposerReasoningLabels(text) {
  let patchedText = text;
  let changed = false;
  for (const patcher of [
    replaceComposerCollapsedReasoningLabel,
    replaceComposerVisibleReasoningLabel,
  ]) {
    const result = patcher(patchedText);
    if (result.state === "patchable") {
      patchedText = result.text;
      changed = true;
      continue;
    }
    if (result.state !== "patched") return result;
  }
  return changed ? { state: "patchable", text: patchedText } : { state: "patched", text };
}

function isReasoningLabelDefaultsEntry(entryPath) {
  return /^webview\/assets\/reasoning-effort-label-messages-.*\.js$/.test(entryPath);
}

function replaceReasoningLabelDefaults(entryPath, text) {
  if (!isReasoningLabelDefaultsEntry(entryPath)) return { state: "missing" };

  let patchedText = text;
  let changed = false;
  for (const [before, after] of REASONING_LABEL_DEFAULT_REPLACEMENTS) {
    const result = replaceExactSameSizeLabel(patchedText, before, after);
    if (result.state === "patchable") {
      patchedText = result.text;
      changed = true;
      continue;
    }
    if (result.state !== "patched") return result;
  }
  return changed ? { state: "patchable", text: patchedText } : { state: "patched", text };
}

function patchEntryBytes(entryPath, bytes) {
  const text = bytes.toString("utf8");

  if (isStableMetadataRetryAsset(text)) {
    const result = replaceStableMetadataNonRetryGuard(text);
    if (result.state === "patched") {
      return {
        state: "patched",
        patchName: "stable-metadata-git-retry",
        additionalPatchNames: text.includes(HISTORY_PROVIDER_FILTER_AFTER)
          ? ["history-provider-filter"]
          : [],
      };
    }
    if (result.state !== "patchable") return result;
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) return { state: "unsafe-size-change" };
    return {
      state: "patchable",
      bytes: patched,
      patchName: "stable-metadata-git-retry",
      additionalPatchNames: result.text.includes(HISTORY_PROVIDER_FILTER_AFTER)
        ? ["history-provider-filter"]
        : [],
    };
  }

  if (isComposerCollapsedReasoningLabelCandidate(entryPath, text)) {
    const result = replaceComposerReasoningLabels(text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "composer-reasoning-labels" };
    }
    if (result.state !== "patchable") return result;
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) return { state: "unsafe-size-change" };
    return {
      state: "patchable",
      bytes: patched,
      patchName: "composer-reasoning-labels",
    };
  }

  if (isReasoningLabelDefaultsEntry(entryPath)) {
    const result = replaceReasoningLabelDefaults(entryPath, text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "reasoning-effort-label-defaults" };
    }
    if (result.state !== "patchable") return result;
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) return { state: "unsafe-size-change" };
    return {
      state: "patchable",
      bytes: patched,
      patchName: "reasoning-effort-label-defaults",
    };
  }

  if (isPowerSliderJsEntry(entryPath)) {
    const result = replacePowerSliderComponent(text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "custom-model-picker-ui" };
    }
    if (result.state !== "patchable") return result;
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) return { state: "unsafe-size-change" };
    return { state: "patchable", bytes: patched, patchName: "custom-model-picker-ui" };
  }

  if (isPowerSliderCssEntry(entryPath)) {
    const result = replacePowerSliderCss(text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "custom-model-picker-ui" };
    }
    if (result.state !== "patchable") return result;
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) return { state: "unsafe-size-change" };
    return { state: "patchable", bytes: patched, patchName: "custom-model-picker-ui" };
  }

  if (
    text.includes(POWER_SELECTIONS_PATCH_MARKER) ||
    PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS.some((marker) => text.includes(marker)) ||
    text.includes(LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER) ||
    containsPowerPickerSignature(text)
  ) {
    const result = replacePowerSelections(text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "reasoning-effort-options" };
    }
    if (result.state !== "patchable") {
      return { state: result.state };
    }
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) {
      return { state: "unsafe-size-change" };
    }
    return { state: "patchable", bytes: patched, patchName: "reasoning-effort-options" };
  }

  if (isPowerSelectionScopeCandidate(text)) {
    const result = replacePowerSelectionScopeCall(text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "reasoning-effort-current-model-scope" };
    }
    if (result.state !== "patchable") {
      return result;
    }
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) {
      return { state: "unsafe-size-change" };
    }
    return {
      state: "patchable",
      bytes: patched,
      patchName: "reasoning-effort-current-model-scope",
    };
  }

  if (entryPath.startsWith("webview/assets/zh-CN-")) {
    const result = replaceZhCnReasoningLabels(entryPath, text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "reasoning-effort-labels-zh-cn" };
    }
    if (result.state === "patchable") {
      const patched = Buffer.from(result.text, "utf8");
      if (patched.length !== bytes.length) {
        return { state: "unsafe-size-change" };
      }
      return { state: "patchable", bytes: patched, patchName: "reasoning-effort-labels-zh-cn" };
    }
    if (result.state === "ambiguous") {
      return result;
    }
  }

  if (
    text.includes("availableModels") &&
    text.includes("useHiddenModels") &&
    text.includes("enabledReasoningEfforts")
  ) {
    const result = replaceModelListFilter(text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "model-picker" };
    }
    if (result.state !== "patchable") {
      return { state: result.state };
    }
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) {
      return { state: "unsafe-size-change" };
    }
    return { state: "patchable", bytes: patched, patchName: "model-picker" };
  }

  if (text.includes("availableModels") && text.includes("useHiddenModels") && text.includes("amazonBedrock")) {
    const result = replaceModelPickerGate(text);
    if (result.state === "patched") {
      return { state: "patched", patchName: "model-picker" };
    }
    if (result.state !== "patchable") {
      return { state: result.state };
    }
    const patched = Buffer.from(result.text, "utf8");
    if (patched.length !== bytes.length) {
      return { state: "unsafe-size-change" };
    }
    return { state: "patchable", bytes: patched, patchName: "model-picker" };
  }

  if (text.includes(HISTORY_PROVIDER_FILTER_AFTER)) {
    return { state: "patched", patchName: "history-provider-filter" };
  }
  if (text.includes(HISTORY_PROVIDER_FILTER_BEFORE)) {
    const patched = Buffer.from(
      text.replaceAll(HISTORY_PROVIDER_FILTER_BEFORE, HISTORY_PROVIDER_FILTER_AFTER),
      "utf8",
    );
    if (patched.length !== bytes.length) {
      return { state: "unsafe-size-change" };
    }
    return { state: "patchable", bytes: patched, patchName: "history-provider-filter" };
  }

  return { state: "missing" };
}

function containsModelPickerSignature(text) {
  return text.includes("availableModels") && text.includes("useHiddenModels");
}

function containsModelPickerGateSignature(text) {
  return (
    containsModelPickerSignature(text) &&
    (text.includes("enabledReasoningEfforts") || text.includes("amazonBedrock"))
  );
}

function verifyEntryPatch(entryPath, bytes) {
  const text = bytes.toString("utf8");
  if (isStableMetadataRetryAsset(text)) {
    return replaceStableMetadataNonRetryGuard(text).state === "patched";
  }
  if (isComposerCollapsedReasoningLabelCandidate(entryPath, text)) {
    return replaceComposerReasoningLabels(text).state === "patched";
  }
  if (isReasoningLabelDefaultsEntry(entryPath)) {
    return replaceReasoningLabelDefaults(entryPath, text).state === "patched";
  }
  if (isPowerSliderJsEntry(entryPath)) {
    return replacePowerSliderComponent(text).state === "patched";
  }
  if (isPowerSliderCssEntry(entryPath)) {
    return replacePowerSliderCss(text).state === "patched";
  }
  if (text.includes(POWER_SELECTIONS_PATCH_MARKER)) {
    return hasCompletePowerSelections(text) && !/](\s*)](?=\}\)\);)/.test(text);
  }
  if (isPowerSelectionScopeCandidate(text)) {
    const call = findPowerSelectionScopeCall(text);
    return call !== null && !call.state && call.selectedModel !== null;
  }
  if (
    entryPath.startsWith("webview/assets/zh-CN-") &&
    text.includes(ZH_CN_MAX_LABEL_AFTER) &&
    text.includes(ZH_CN_ULTRA_LABEL_AFTER)
  ) {
    return true;
  }
  if (
    text.includes("availableModels") &&
    text.includes("useHiddenModels") &&
    text.includes("enabledReasoningEfforts")
  ) {
    return (
      (APPLIED_NEEDLES.some((needle) => needle.test(text)) || isCompactedModelPickerGate(text)) &&
      MAX_REASONING_EFFORT_FILTER_AFTER.test(text)
    );
  }
  if (text.includes("availableModels") && text.includes("useHiddenModels") && text.includes("amazonBedrock")) {
    return APPLIED_NEEDLES.some((needle) => needle.test(text));
  }
  if (text.includes(HISTORY_PROVIDER_FILTER_BEFORE)) {
    return false;
  }
  return true;
}

function candidateAsarFiles(files) {
  const preferred = files.filter((file) => /^webview\/assets\/model-list-filter-.*\.js$/.test(file.path));
  if (preferred.length) {
    return preferred;
  }
  return files.filter(
    (file) => file.path.startsWith("webview/assets/") && file.path.endsWith(".js") && file.entry.size <= 20000,
  );
}

function patchCandidateAsarFiles(files) {
  const preferred = candidateAsarFiles(files);
  const seen = new Set(preferred.map((file) => file.path));
  const uiAssets = files.filter(
    (file) =>
      (isPowerSliderJsEntry(file.path) || isPowerSliderCssEntry(file.path)) &&
      !seen.has(file.path),
  );
  uiAssets.forEach((file) => seen.add(file.path));
  const jsFiles = files.filter((file) => file.path.endsWith(".js") && !seen.has(file.path));
  return [...preferred, ...uiAssets, ...jsFiles];
}

function isWindowsAppsPath(filePath) {
  return /\\WindowsApps\\/i.test(path.resolve(filePath));
}

function appRootFromAsar(appAsarPath) {
  return path.dirname(path.dirname(appAsarPath));
}

function integrityBlockSize(integrity) {
  const blockSize = Number(integrity.blockSize ?? 4194304);
  if (!Number.isSafeInteger(blockSize) || blockSize <= 0) {
    throw new Error(`Invalid ASAR integrity blockSize: ${integrity.blockSize}`);
  }
  return blockSize;
}

function updateEntryIntegrity(entry, bytes) {
  if (!entry.integrity || typeof entry.integrity !== "object") {
    return;
  }
  const blockSize = Number(entry.integrity.blockSize || 4194304);
  const blocks = [];
  for (let offset = 0; offset < bytes.length; offset += blockSize) {
    blocks.push(sha256(bytes.subarray(offset, offset + blockSize)));
  }
  if (!blocks.length) {
    blocks.push(sha256(Buffer.alloc(0)));
  }
  entry.integrity.algorithm = entry.integrity.algorithm || "SHA256";
  entry.integrity.hash = sha256(bytes);
  entry.integrity.blocks = blocks;
}

function verifyEntryIntegrity(entry, bytes) {
  if (!entry.integrity || typeof entry.integrity !== "object") return true;
  let blockSize;
  try {
    blockSize = integrityBlockSize(entry.integrity);
  } catch {
    return false;
  }
  if (
    entry.integrity.algorithm &&
    String(entry.integrity.algorithm).toUpperCase() !== "SHA256"
  ) {
    return false;
  }
  const blocks = [];
  for (let offset = 0; offset < bytes.length; offset += blockSize) {
    blocks.push(sha256(bytes.subarray(offset, offset + blockSize)));
  }
  if (!blocks.length) blocks.push(sha256(Buffer.alloc(0)));
  return (
    entry.integrity.hash === sha256(bytes) &&
    Array.isArray(entry.integrity.blocks) &&
    entry.integrity.blocks.length === blocks.length &&
    entry.integrity.blocks.every((hash, index) => hash === blocks[index])
  );
}

function writeFully(fd, bytes, position) {
  let written = 0;
  while (written < bytes.length) {
    const count = fs.writeSync(
      fd,
      bytes,
      written,
      bytes.length - written,
      position + written,
    );
    if (count <= 0) throw new Error(`Short write at file offset ${position + written}.`);
    written += count;
  }
}

function writePatchedAsar(asarPath, asar, targets) {
  for (const target of targets) {
    updateEntryIntegrity(target.entry, target.bytes);
  }

  const newHeaderBytes = Buffer.from(JSON.stringify(asar.header), "utf8");
  if (newHeaderBytes.length !== asar.headerBytes.length) {
    throw new Error("ASAR header length changed; refusing to patch in place.");
  }

  const fd = fs.openSync(asarPath, "r+");
  try {
    for (const target of targets) {
      writeFully(fd, target.bytes, target.offset);
    }
    writeFully(fd, newHeaderBytes, 16);
    fs.fsyncSync(fd);
  } finally {
    fs.closeSync(fd);
  }

  return {
    headerHash: sha256(newHeaderBytes),
    patchedFiles: targets.map((target) => target.path),
  };
}

function requireEnv(name) {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`Missing required environment variable: ${name}`);
  }
  return value;
}

function realpathIfExists(filePath) {
  return fs.realpathSync(filePath);
}

function normalizeForCompare(filePath) {
  return path.resolve(filePath).replaceAll("/", path.sep).toLowerCase();
}

function pathInside(child, parent) {
  const relative = path.relative(parent, child);
  return relative === "" || (!!relative && !relative.startsWith("..") && !path.isAbsolute(relative));
}

function assertControlledMarker(markerPath, realWorkspace, realRoot, realAsar) {
  if (!fs.existsSync(markerPath)) {
    throw new Error(`Missing CodexDeck controlled marker: ${markerPath}`);
  }
  const marker = JSON.parse(fs.readFileSync(markerPath, "utf8"));
  const markerWorkspace = marker.workspace ? realpathIfExists(marker.workspace) : "";
  const markerRoot = marker.controlledAppRoot ? realpathIfExists(marker.controlledAppRoot) : "";
  const markerAsar = marker.controlledAppAsarPath ? realpathIfExists(marker.controlledAppAsarPath) : "";
  const markerExe = marker.controlledExePath ? realpathIfExists(marker.controlledExePath) : "";
  if (
    normalizeForCompare(markerWorkspace) !== normalizeForCompare(realWorkspace) ||
    normalizeForCompare(markerRoot) !== normalizeForCompare(realRoot) ||
    normalizeForCompare(markerAsar) !== normalizeForCompare(realAsar)
  ) {
    throw new Error("CodexDeck controlled marker does not match the requested target.");
  }
  if (!markerExe || !pathInside(markerExe, realRoot)) {
    throw new Error("CodexDeck controlled marker is missing a valid launch path.");
  }
  return { launchPath: markerExe };
}

function targetPaths() {
  const workspaceDir = requireEnv("CODEXDECK_MULTIMODEL_WORKSPACE_DIR");
  const controlledAppAsar = requireEnv("CODEXDECK_CONTROLLED_APP_ASAR");
  const controlledAppRoot = process.env.CODEXDECK_CONTROLLED_APP_ROOT?.trim() || appRootFromAsar(controlledAppAsar);

  if (!fs.existsSync(workspaceDir)) {
    throw new Error(`Missing multi-model workspace: ${workspaceDir}`);
  }
  if (!fs.existsSync(controlledAppRoot)) {
    throw new Error(`Missing controlled Codex app root: ${controlledAppRoot}`);
  }
  if (!fs.existsSync(controlledAppAsar)) {
    throw new Error(`Missing controlled app.asar: ${controlledAppAsar}`);
  }

  const realWorkspace = realpathIfExists(workspaceDir);
  const realRoot = realpathIfExists(controlledAppRoot);
  const realAsar = realpathIfExists(controlledAppAsar);
  const expectedAsar = path.join(realRoot, "resources", "app.asar");
  const requestedPatchStatePath = process.env.CODEXDECK_PATCH_STATE_PATH?.trim();
  const patchStatePath = path.resolve(
    requestedPatchStatePath || path.join(realWorkspace, "model-picker-patch-state.json"),
  );
  const realPatchStateParent = realpathIfExists(path.dirname(patchStatePath));

  if (isWindowsAppsPath(realRoot) || isWindowsAppsPath(realAsar)) {
    throw new Error("Refusing to patch a WindowsApps Codex install; only controlled copies are supported.");
  }
  if (!pathInside(realRoot, realWorkspace)) {
    throw new Error("Controlled Codex root is outside the multi-model workspace.");
  }
  if (!pathInside(realAsar, realRoot)) {
    throw new Error("Controlled app.asar is outside the controlled Codex root.");
  }
  if (normalizeForCompare(realAsar) !== normalizeForCompare(expectedAsar)) {
    throw new Error(`Controlled app.asar must be ${expectedAsar}`);
  }
  if (!pathInside(realPatchStateParent, realWorkspace)) {
    throw new Error("Patch state path is outside the multi-model workspace.");
  }
  if (fs.existsSync(patchStatePath)) {
    const realPatchState = realpathIfExists(patchStatePath);
    if (normalizeForCompare(realPatchState) !== normalizeForCompare(patchStatePath)) {
      throw new Error("Patch state path must not be a symbolic link.");
    }
  }

  const marker = assertControlledMarker(
    path.join(realRoot, ".codexdeck-controlled.json"),
    realWorkspace,
    realRoot,
    realAsar,
  );

  return {
    appAsarPath: realAsar,
    backupDir: path.join(realWorkspace, "patch-backups"),
    patchStatePath,
    controlledAppRoot: realRoot,
    launchPath: marker.launchPath,
  };
}

function backupFile(filePath, backupDir, label) {
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  fs.mkdirSync(backupDir, { recursive: true });
  const backupPath = path.join(backupDir, `app.asar.${label}.${stamp}.bak`);
  fs.copyFileSync(filePath, backupPath);
  return backupPath;
}

function writeFileAtomic(filePath, bytes) {
  const parent = path.dirname(filePath);
  fs.mkdirSync(parent, { recursive: true });
  const temporaryPath = path.join(
    parent,
    `.${path.basename(filePath)}.tmp-${process.pid}-${crypto.randomBytes(8).toString("hex")}`,
  );
  try {
    fs.writeFileSync(temporaryPath, bytes);
    fs.renameSync(temporaryPath, filePath);
  } finally {
    fs.rmSync(temporaryPath, { force: true });
  }
}

function writePatchState(patchStatePath, state) {
  writeFileAtomic(patchStatePath, Buffer.from(`${JSON.stringify(state, null, 2)}\n`, "utf8"));
}

function restorePatchState(patchStatePath, previousBytes) {
  if (previousBytes === null) {
    fs.rmSync(patchStatePath, { force: true });
    return;
  }
  if (fs.existsSync(patchStatePath)) {
    const currentBytes = fs.readFileSync(patchStatePath);
    if (currentBytes.equals(previousBytes)) return;
  }
  writeFileAtomic(patchStatePath, previousBytes);
}

function main() {
  const {
    appAsarPath,
    backupDir,
    patchStatePath,
    controlledAppRoot = "",
    launchPath = "",
  } = targetPaths();

  if (!fs.existsSync(appAsarPath)) {
    throw new Error(`Missing app.asar: ${appAsarPath}`);
  }

  const targetAsarHashBeforePatch = sha256(fs.readFileSync(appAsarPath));
  const sourceAsarHash = sourceAsarHashFromEnv || targetAsarHashBeforePatch;
  const asar = readAsarHeader(appAsarPath);
  const files = patchCandidateAsarFiles(listAsarFiles(asar.header));
  const targets = [];
  const patchNames = new Set();
  const alreadyPatchNames = new Set();
  let sawModelPicker = false;
  let sawPowerPicker = false;
  let sawPowerSelectionScope = false;
  let sawPowerSliderJs = false;
  let sawPowerSliderCss = false;
  let sawComposerCollapsedReasoningLabel = false;
  let sawReasoningLabelDefaults = false;
  let sawStableMetadataRetry = false;

  for (const file of files) {
    const { offset, bytes } = readAsarEntry(appAsarPath, asar.filesOffset, file.entry);
    const text = bytes.toString("utf8");
    const result = patchEntryBytes(file.path, bytes);

    const isPowerPicker =
      text.includes(POWER_SELECTIONS_PATCH_MARKER) ||
      PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS.some((marker) => text.includes(marker)) ||
      text.includes(LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER) ||
      containsPowerPickerSignature(text);
    const isPowerSelectionScope = isPowerSelectionScopeCandidate(text);
    const isPowerSliderJs = isPowerSliderJsEntry(file.path);
    const isPowerSliderCss = isPowerSliderCssEntry(file.path);
    const isComposerCollapsedReasoningLabel = isComposerCollapsedReasoningLabelCandidate(
      file.path,
      text,
    );
    const isReasoningLabelDefaults = isReasoningLabelDefaultsEntry(file.path);
    const isStableMetadataRetry = isStableMetadataRetryAsset(text);
    const isPreferredModelPickerEntry = /^webview\/assets\/model-list-filter-.*\.js$/.test(
      file.path,
    );
    sawPowerPicker ||= isPowerPicker;
    sawPowerSelectionScope ||= isPowerSelectionScope;
    sawPowerSliderJs ||= isPowerSliderJs;
    sawPowerSliderCss ||= isPowerSliderCss;
    sawComposerCollapsedReasoningLabel ||= isComposerCollapsedReasoningLabel;
    sawReasoningLabelDefaults ||= isReasoningLabelDefaults;
    sawStableMetadataRetry ||= isStableMetadataRetry;
    if (
      isStableMetadataRetry &&
      (result.patchName !== "stable-metadata-git-retry" ||
        ["ambiguous", "missing", "unsafe-size-change", "unsupported-preimage"].includes(
          result.state,
        ))
    ) {
      throw new Error(`Stable metadata Git retry patch failed for ${file.path}: ${result.state}`);
    }
    if (
      result.patchName === "model-picker" ||
      (isPreferredModelPickerEntry && containsModelPickerGateSignature(text))
    ) {
      sawModelPicker = true;
      if (
        ["ambiguous", "missing", "unsafe-size-change", "unsupported-preimage"].includes(
          result.state,
        )
      ) {
        throw new Error(`Model picker patch failed for ${file.path}: ${result.state}`);
      }
    }
    if (
      isPowerPicker &&
      ["ambiguous", "missing", "unsafe-size-change", "unsupported-preimage"].includes(result.state)
    ) {
      throw new Error(`Reasoning effort patch failed for ${file.path}: ${result.state}`);
    }
    if (
      isPowerSelectionScope &&
      ["ambiguous", "missing", "unsafe-size-change", "unsupported-preimage"].includes(result.state)
    ) {
      throw new Error(`Reasoning effort scope patch failed for ${file.path}: ${result.state}`);
    }
    if (
      (isPowerSliderJs || isPowerSliderCss) &&
      ["ambiguous", "missing", "unsafe-size-change", "unsupported-preimage"].includes(result.state)
    ) {
      throw new Error(`Custom model picker UI patch failed for ${file.path}: ${result.state}`);
    }
    if (
      isComposerCollapsedReasoningLabel &&
      (result.patchName !== "composer-reasoning-labels" ||
        ["ambiguous", "missing", "unsafe-size-change", "unsupported-preimage"].includes(
          result.state,
        ))
    ) {
      throw new Error(`Composer reasoning label patch failed for ${file.path}: ${result.state}`);
    }
    if (
      isReasoningLabelDefaults &&
      (result.patchName !== "reasoning-effort-label-defaults" ||
        ["ambiguous", "missing", "unsafe-size-change", "unsupported-preimage"].includes(
          result.state,
        ))
    ) {
      throw new Error(`Reasoning label defaults patch failed for ${file.path}: ${result.state}`);
    }

    if (result.state === "patched" && result.patchName) {
      if (!verifyEntryIntegrity(file.entry, bytes)) {
        throw new Error(`Patched ASAR integrity mismatch for ${file.path}.`);
      }
      alreadyPatchNames.add(result.patchName);
      result.additionalPatchNames?.forEach((name) => alreadyPatchNames.add(name));
    }

    if (result.state === "patchable") {
      targets.push({
        path: file.path,
        entry: file.entry,
        offset,
        bytes: result.bytes,
      });
      patchNames.add(result.patchName);
      result.additionalPatchNames?.forEach((name) => patchNames.add(name));
    }
  }

  if (!sawModelPicker) {
    throw new Error("Did not find Codex model picker gate in app.asar.");
  }
  if (!patchNames.has("model-picker") && !alreadyPatchNames.has("model-picker")) {
    throw new Error("Did not patch or verify the Codex model picker gate in app.asar.");
  }
  if (sawPowerPicker && !sawPowerSelectionScope) {
    throw new Error("Did not find the model picker Power slider scope call in app.asar.");
  }
  if (!sawPowerSliderJs || !sawPowerSliderCss) {
    throw new Error("Did not find both model picker Power slider JS and CSS assets in app.asar.");
  }
  if (!sawComposerCollapsedReasoningLabel || !sawReasoningLabelDefaults) {
    throw new Error("Did not find both composer and default reasoning label assets in app.asar.");
  }
  if (
    !sawStableMetadataRetry ||
    (!patchNames.has("stable-metadata-git-retry") &&
      !alreadyPatchNames.has("stable-metadata-git-retry"))
  ) {
    throw new Error("Did not patch or verify the stable metadata Git retry guard in app.asar.");
  }

  if (!targets.length) {
    const patchedAsarHash = sha256(fs.readFileSync(appAsarPath));
    const state = {
      status: "already-patched",
      patchVersion: PATCH_VERSION,
      mode: "controlled",
      appAsarPath,
      controlledAppRoot,
      launchPath,
      sourceAsarHash,
      sourceCodexVersion,
      patchedAsarHash,
      patchNames: [...new Set([...alreadyPatchNames, ...patchNames])],
      patchedAt: new Date().toISOString(),
    };
    writePatchState(patchStatePath, state);
    console.log(
      JSON.stringify(state, null, 2),
    );
    return;
  }

  const backupPath = backupFile(appAsarPath, backupDir, "controlled");
  const previousPatchState = fs.existsSync(patchStatePath)
    ? fs.readFileSync(patchStatePath)
    : null;
  let writeResult;
  let state;
  try {
    writeResult = writePatchedAsar(appAsarPath, asar, targets);
    const verificationAsar = readAsarHeader(appAsarPath);
    const verificationFiles = listAsarFiles(verificationAsar.header);
    for (const target of targets) {
      const file = verificationFiles.find((candidate) => candidate.path === target.path);
      if (!file) {
        throw new Error(`Patch verification failed for ${target.path}: missing patched file`);
      }
      const { bytes } = readAsarEntry(appAsarPath, verificationAsar.filesOffset, file.entry);
      if (!verifyEntryPatch(file.path, bytes)) {
        throw new Error(`Patch verification failed for ${target.path}`);
      }
      if (!verifyEntryIntegrity(file.entry, bytes)) {
        throw new Error(`Patch integrity verification failed for ${target.path}`);
      }
    }
    const patchedAsarHash = sha256(fs.readFileSync(appAsarPath));
    state = {
      status: "patched",
      patchVersion: PATCH_VERSION,
      mode: "controlled",
      appAsarPath,
      controlledAppRoot,
      launchPath,
      backupPath,
      sourceAsarHash,
      sourceCodexVersion,
      patchedAsarHash,
      patchNames: [...new Set([...alreadyPatchNames, ...patchNames])],
      patchedFiles: writeResult.patchedFiles,
      patchedHeaderHash: writeResult.headerHash,
      patchedAt: new Date().toISOString(),
    };
    writePatchState(patchStatePath, state);
  } catch (error) {
    const restoreErrors = [];
    try {
      fs.copyFileSync(backupPath, appAsarPath);
    } catch (restoreError) {
      restoreErrors.push(restoreError);
    }
    try {
      restorePatchState(patchStatePath, previousPatchState);
    } catch (restoreError) {
      restoreErrors.push(restoreError);
    }
    if (restoreErrors.length > 0) {
      throw new Error(
        `Patch failed and backup restore failed: ${error instanceof Error ? error.message : error}; ` +
          restoreErrors
            .map((restoreError) =>
              restoreError instanceof Error ? restoreError.message : String(restoreError),
            )
            .join("; "),
        { cause: error },
      );
    }
    throw new Error(
      `Patch failed; restored the controlled app.asar backup: ${error instanceof Error ? error.message : error}`,
      { cause: error },
    );
  }
  console.log(JSON.stringify(state, null, 2));
}

main();
