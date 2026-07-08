import { HolderOutlined } from "@ant-design/icons";
import { Button, Tooltip } from "antd";
import { type ReactNode, useMemo } from "react";
import {
  closestCenter,
  DndContext,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { SortableContext, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";

type SortableModelCatalogScopeProps = {
  children: ReactNode;
  enabled: boolean;
  items: string[];
  onMove: (fromIndex: number, toIndex: number) => void;
};

type SortableModelCatalogRowProps = {
  children: (sortHandle: ReactNode) => ReactNode;
  id: string;
  sortingEnabled: boolean;
};

export function SortableModelCatalogScope({
  children,
  enabled,
  items,
  onMove,
}: SortableModelCatalogScopeProps) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 6 },
    }),
  );

  const handleDragEnd = (event: DragEndEvent) => {
    if (!enabled || !event.over || event.active.id === event.over.id) {
      return;
    }

    const fromIndex = items.indexOf(String(event.active.id));
    const toIndex = items.indexOf(String(event.over.id));
    if (fromIndex === -1 || toIndex === -1) {
      return;
    }

    onMove(fromIndex, toIndex);
  };

  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <SortableContext items={items} strategy={verticalListSortingStrategy}>
        {children}
      </SortableContext>
    </DndContext>
  );
}

export function SortableModelCatalogRow({
  children,
  id,
  sortingEnabled,
}: SortableModelCatalogRowProps) {
  const {
    attributes,
    isDragging,
    listeners,
    setActivatorNodeRef,
    setNodeRef,
    transform,
    transition,
  } = useSortable({ id, disabled: !sortingEnabled });
  const style = useMemo(
    () => ({
      transform: CSS.Transform.toString(transform),
      transition,
    }),
    [transform, transition],
  );
  const sortHandle = sortingEnabled ? (
    <Tooltip title="拖动排序">
      <Button
        ref={setActivatorNodeRef}
        type="text"
        size="small"
        className="apiModelCatalogDragHandle"
        icon={<HolderOutlined />}
        aria-label="拖动排序"
        {...attributes}
        {...listeners}
      />
    </Tooltip>
  ) : null;

  return (
    <div
      ref={setNodeRef}
      className={`sortableModelCatalogRow${isDragging ? " isDragging" : ""}`}
      style={style}
    >
      {children(sortHandle)}
    </div>
  );
}
