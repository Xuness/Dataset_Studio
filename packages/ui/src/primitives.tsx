import { useEffect, useRef, useId } from "react";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { RotateCcw, X } from "lucide-react";
export function Button({
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return <button {...props} className={"button " + className} />;
}
export function EmptyState({
  title,
  children,
  icon,
}: {
  title: string;
  children: ReactNode;
  icon?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <div className="empty-icon">{icon}</div>
      <h2>{title}</h2>
      <div>{children}</div>
    </div>
  );
}
/** A labelled property. `onReset` shows the editor reset arrow; pass it only
 * while the value differs from its default. */
export function Field({
  label,
  children,
  onReset,
}: {
  label: string;
  children: ReactNode;
  onReset?: (() => void) | undefined;
}) {
  return (
    <label className={"field" + (onReset ? " field-changed" : "")}>
      <span>{label}</span>
      {children}
      {onReset && <ResetButton label={label} onReset={onReset} />}
    </label>
  );
}
export function ResetButton({
  label,
  onReset,
}: {
  label: string;
  onReset: () => void;
}) {
  return (
    <button
      type="button"
      className="field-reset"
      title="重置为默认值"
      aria-label={"将" + label + "重置为默认值"}
      onClick={(event) => {
        event.preventDefault();
        onReset();
      }}
    >
      <RotateCcw size={12} />
    </button>
  );
}
export function Dialog({
  title,
  children,
  onClose,
  className = "",
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  className?: string;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);
  return (
    <dialog
      ref={ref}
      className={"dialog " + className}
      aria-labelledby={titleId}
      onCancel={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="dialog-content">
        <header>
          <h2 id={titleId}>{title}</h2>
          <button aria-label="关闭" className="icon-button" onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        {children}
      </div>
    </dialog>
  );
}
