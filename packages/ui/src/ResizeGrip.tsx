import { useRef } from "react";

export function ResizeGrip({
  label,
  orientation,
  value,
  minimum,
  maximum,
  onChange,
  onReset,
  reverse = false,
}: {
  label: string;
  orientation: "vertical" | "horizontal";
  value: number;
  minimum: number;
  maximum: number;
  onChange: (value: number) => void;
  onReset: () => void;
  reverse?: boolean;
}) {
  const drag = useRef<{
    pointer: number;
    origin: number;
    value: number;
  } | null>(null);
  const clamp = (next: number) =>
    Math.max(minimum, Math.min(maximum, Math.round(next)));
  return (
    <div
      className={"resize-grip " + orientation}
      role="separator"
      tabIndex={0}
      aria-label={label}
      aria-orientation={orientation}
      aria-valuenow={value}
      aria-valuemin={minimum}
      aria-valuemax={maximum}
      title={label + "；拖动调整，双击恢复"}
      onDoubleClick={onReset}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = {
          pointer: e.pointerId,
          origin: orientation === "vertical" ? e.clientX : e.clientY,
          value,
        };
      }}
      onPointerMove={(e) => {
        const start = drag.current;
        if (!start || start.pointer !== e.pointerId) return;
        const distance =
          (orientation === "vertical" ? e.clientX : e.clientY) - start.origin;
        onChange(clamp(start.value + distance * (reverse ? -1 : 1)));
      }}
      onPointerUp={(e) => {
        if (drag.current?.pointer === e.pointerId) {
          drag.current = null;
          e.currentTarget.releasePointerCapture(e.pointerId);
        }
      }}
      onLostPointerCapture={() => {
        drag.current = null;
      }}
      onKeyDown={(e) => {
        const delta = (
          {
            ArrowLeft: -16,
            ArrowRight: 16,
            ArrowUp: -16,
            ArrowDown: 16,
          } as Record<string, number>
        )[e.key];
        if (delta !== undefined) {
          e.preventDefault();
          onChange(clamp(value + delta * (reverse ? -1 : 1)));
        }
        if (e.key === "Home") {
          e.preventDefault();
          onReset();
        }
      }}
    />
  );
}
