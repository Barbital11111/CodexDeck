import type { RelayModelCatalogEntry } from "../types/app";

let modelCatalogRowIdSeed = 0;

export function createModelCatalogRowId(prefix: string) {
  modelCatalogRowIdSeed += 1;
  return `${prefix}-${Date.now()}-${modelCatalogRowIdSeed}`;
}

export function createModelCatalogRowIds(count: number, prefix: string) {
  return Array.from({ length: count }, () => createModelCatalogRowId(prefix));
}

function modelCatalogSortKey(entry: RelayModelCatalogEntry) {
  return (entry.displayName?.trim() || entry.model.trim()).toLocaleLowerCase();
}

export function sortRelayModelCatalog(entries: RelayModelCatalogEntry[]) {
  return [...entries].sort((left, right) => {
    const labelDiff = modelCatalogSortKey(left).localeCompare(
      modelCatalogSortKey(right),
      "zh-CN",
      {
        numeric: true,
        sensitivity: "base",
      },
    );
    if (labelDiff !== 0) {
      return labelDiff;
    }
    return left.model.trim().localeCompare(right.model.trim(), "zh-CN", {
      numeric: true,
      sensitivity: "base",
    });
  });
}

export function moveArrayItem<T>(
  entries: T[],
  fromIndex: number,
  toIndex: number,
) {
  if (
    fromIndex === toIndex ||
    fromIndex < 0 ||
    toIndex < 0 ||
    fromIndex >= entries.length ||
    toIndex >= entries.length
  ) {
    return entries;
  }

  const nextEntries = [...entries];
  const [movedEntry] = nextEntries.splice(fromIndex, 1);
  if (!movedEntry) {
    return entries;
  }

  nextEntries.splice(toIndex, 0, movedEntry);
  return nextEntries;
}

export function moveRelayModelCatalogEntry(
  entries: RelayModelCatalogEntry[],
  fromIndex: number,
  toIndex: number,
) {
  return moveArrayItem(entries, fromIndex, toIndex);
}
