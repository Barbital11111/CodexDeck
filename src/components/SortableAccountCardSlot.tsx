import { HolderOutlined } from "@ant-design/icons";
import { Button, Tooltip } from "antd";
import { type ReactNode, useMemo } from "react";
import { CSS } from "@dnd-kit/utilities";
import { useSortable } from "@dnd-kit/sortable";

type SortableAccountCardSlotProps = {
  id: string;
  children: ReactNode | ((sortHandle: ReactNode) => ReactNode);
  className?: string;
  enabled?: boolean;
  handleVariant?: "button" | "bar";
  handleLabel?: string;
};

export function SortableAccountCardSlot({
  id,
  children,
  className = "accountCardSlot",
  enabled = true,
  handleVariant = "button",
  handleLabel = "拖动排序",
}: SortableAccountCardSlotProps) {
  const {
    attributes,
    isDragging,
    listeners,
    setActivatorNodeRef,
    setNodeRef,
    transform,
    transition,
  } = useSortable({ id, disabled: !enabled });
  const sortHandle = enabled ? (
    <Tooltip title={handleLabel}>
      <Button
        ref={setActivatorNodeRef}
        type="text"
        size="small"
        className={
          handleVariant === "bar"
            ? "accountCardSortBar"
            : "accountCardSortHandle"
        }
        icon={handleVariant === "button" ? <HolderOutlined /> : undefined}
        aria-label={handleLabel}
        {...attributes}
        {...listeners}
      >
        {handleVariant === "bar" ? (
          <span className="accountCardSortBarGrip" aria-hidden="true" />
        ) : null}
      </Button>
    </Tooltip>
  ) : null;
  const style = useMemo(
    () => ({
      transform: CSS.Transform.toString(transform),
      transition,
    }),
    [transform, transition],
  );
  const resolvedChildren =
    typeof children === "function" ? children(sortHandle) : children;

  return (
    <div
      ref={setNodeRef}
      className={`${className} sortableAccountCardSlot${isDragging ? " isSorting" : ""}`}
      style={style}
    >
      {resolvedChildren}
    </div>
  );
}
