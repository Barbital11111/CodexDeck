import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
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

const PATCH_VERSION = "model-picker-v2";
const sourceCodexVersion = process.env.CODEXDECK_SOURCE_CODEX_VERSION?.trim() || null;
const sourceAsarHashFromEnv = process.env.CODEXDECK_SOURCE_ASAR_HASH?.trim() || null;

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function readAsarHeader(asarPath) {
  const fd = fs.openSync(asarPath, "r");
  try {
    const prefix = Buffer.alloc(16);
    fs.readSync(fd, prefix, 0, prefix.length, 0);
    const headerJsonSize = prefix.readUInt32LE(12);
    const headerBytes = Buffer.alloc(headerJsonSize);
    fs.readSync(fd, headerBytes, 0, headerJsonSize, 16);
    return {
      headerJsonSize,
      headerBytes,
      header: JSON.parse(headerBytes.toString("utf8")),
      filesOffset: 16 + headerJsonSize,
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
  for (const exact of EXACT_PATCHES) {
    if (text.includes(exact.after)) {
      return { state: "patched", text };
    }
    const count = text.split(exact.before).length - 1;
    if (count === 1) {
      return { state: "patchable", text: text.replace(exact.before, exact.after) };
    }
    if (count > 1) {
      return { state: "ambiguous" };
    }
  }

  if (APPLIED_NEEDLES.some((needle) => needle.test(text))) {
    return { state: "patched", text };
  }

  for (const needle of PATCH_NEEDLES) {
    const matches = [...text.matchAll(needle)];
    if (matches.length === 1) {
      return {
        state: "patchable",
        text: text.replace(needle, (_match, prefix, _gateVar, suffix) => `${prefix}0${suffix}`),
      };
    }
    if (matches.length > 1) {
      return { state: "ambiguous" };
    }
  }

  return { state: "missing" };
}

function patchEntryBytes(_entryPath, bytes) {
  const text = bytes.toString("utf8");

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

function verifyEntryPatch(_entryPath, bytes) {
  const text = bytes.toString("utf8");
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
  const jsFiles = files.filter((file) => file.path.endsWith(".js") && !seen.has(file.path));
  return [...preferred, ...jsFiles];
}

function isWindowsAppsPath(filePath) {
  return /\\WindowsApps\\/i.test(path.resolve(filePath));
}

function appRootFromAsar(appAsarPath) {
  return path.dirname(path.dirname(appAsarPath));
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
    fs.writeSync(fd, newHeaderBytes, 0, newHeaderBytes.length, 16);
    for (const target of targets) {
      fs.writeSync(fd, target.bytes, 0, target.bytes.length, target.offset);
    }
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

  const marker = assertControlledMarker(
    path.join(realRoot, ".codexdeck-controlled.json"),
    realWorkspace,
    realRoot,
    realAsar,
  );

  return {
    appAsarPath: realAsar,
    backupDir: path.join(realWorkspace, "patch-backups"),
    patchStatePath: path.join(realWorkspace, "model-picker-patch-state.json"),
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

  for (const file of files) {
    const { offset, bytes } = readAsarEntry(appAsarPath, asar.filesOffset, file.entry);
    const text = bytes.toString("utf8");
    const result = patchEntryBytes(file.path, bytes);

    if (
      result.patchName === "model-picker" ||
      result.state === "ambiguous" ||
      result.state === "unsafe-size-change" ||
      containsModelPickerSignature(text)
    ) {
      sawModelPicker = true;
      if (result.state === "ambiguous" || result.state === "unsafe-size-change") {
        throw new Error(`Model picker patch failed for ${file.path}: ${result.state}`);
      }
    }

    if (result.state === "patched" && result.patchName) {
      alreadyPatchNames.add(result.patchName);
    }

    if (result.state === "patchable") {
      targets.push({
        path: file.path,
        entry: file.entry,
        offset,
        bytes: result.bytes,
      });
      patchNames.add(result.patchName);
    }
  }

  if (!sawModelPicker) {
    throw new Error("Did not find Codex model picker gate in app.asar.");
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
      patchNames: [...alreadyPatchNames],
      patchedAt: new Date().toISOString(),
    };
    fs.mkdirSync(path.dirname(patchStatePath), { recursive: true });
    fs.writeFileSync(patchStatePath, `${JSON.stringify(state, null, 2)}\n`, "utf8");
    console.log(
      JSON.stringify(state, null, 2),
    );
    return;
  }

  const backupPath = backupFile(appAsarPath, backupDir, "controlled");
  const writeResult = writePatchedAsar(appAsarPath, asar, targets);
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
  }
  const patchedAsarHash = sha256(fs.readFileSync(appAsarPath));
  const state = {
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
    patchNames: [...patchNames],
    patchedFiles: writeResult.patchedFiles,
    patchedHeaderHash: writeResult.headerHash,
    patchedAt: new Date().toISOString(),
  };
  fs.writeFileSync(patchStatePath, `${JSON.stringify(state, null, 2)}\n`, "utf8");
  console.log(JSON.stringify(state, null, 2));
}

main();
