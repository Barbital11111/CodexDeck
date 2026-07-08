export function sortBySavedAccountOrder<T>(
  items: T[],
  order: string[] | null | undefined,
  getKey: (item: T) => string,
  fallbackCompare: (left: T, right: T) => number,
) {
  const orderIndex = new Map((order ?? []).map((key, index) => [key, index]));
  return [...items].sort((left, right) => {
    const leftIndex = orderIndex.get(getKey(left));
    const rightIndex = orderIndex.get(getKey(right));
    if (leftIndex !== undefined && rightIndex !== undefined && leftIndex !== rightIndex) {
      return leftIndex - rightIndex;
    }
    if (leftIndex !== undefined) {
      return -1;
    }
    if (rightIndex !== undefined) {
      return 1;
    }
    return fallbackCompare(left, right);
  });
}

export function moveAccountKeyAround(
  currentKeys: string[],
  draggedKey: string,
  targetKey: string,
  placement: "before" | "after",
) {
  if (draggedKey === targetKey || !currentKeys.includes(draggedKey)) {
    return currentKeys;
  }

  const withoutDragged = currentKeys.filter((key) => key !== draggedKey);
  const targetIndex = withoutDragged.indexOf(targetKey);
  if (targetIndex < 0) {
    return currentKeys;
  }
  const insertIndex = placement === "after" ? targetIndex + 1 : targetIndex;

  return [
    ...withoutDragged.slice(0, insertIndex),
    draggedKey,
    ...withoutDragged.slice(insertIndex),
  ];
}

export function moveAccountKeyToTarget(
  currentKeys: string[],
  activeKey: string,
  overKey: string,
) {
  const oldIndex = currentKeys.indexOf(activeKey);
  const newIndex = currentKeys.indexOf(overKey);
  if (oldIndex < 0 || newIndex < 0 || oldIndex === newIndex) {
    return currentKeys;
  }

  const nextKeys = [...currentKeys];
  const [movedKey] = nextKeys.splice(oldIndex, 1);
  if (!movedKey) {
    return currentKeys;
  }
  nextKeys.splice(newIndex, 0, movedKey);
  return nextKeys;
}

export function dragPlacementForTarget(
  event: { clientX: number; clientY: number; currentTarget: HTMLElement },
): "before" | "after" {
  const rect = event.currentTarget.getBoundingClientRect();
  const isLowerHalf = event.clientY > rect.top + rect.height / 2;
  const isRightHalf = event.clientX > rect.left + rect.width / 2;
  return isLowerHalf || isRightHalf ? "after" : "before";
}
