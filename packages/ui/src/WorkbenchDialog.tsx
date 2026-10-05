import { createContext, useContext, useEffect, useRef } from "react";
import type { ReactNode } from "react";
import { X } from "lucide-react";

/**
 * "panel" renders the next WorkbenchDialog inline, for a host that places it
 * in a dock panel; dialogs nested inside it are modal again.
 */
export const WorkbenchDialogMode = createContext<"modal" | "panel">("modal");

export function WorkbenchDialog({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const mode = useContext(WorkbenchDialogMode);
  return mode === "panel" ? (
    <section
      className="wb-dialog wb-dialog-panel"
      role="dialog"
      aria-modal="false"
      aria-label={title}
    >
      <WorkbenchDialogMode.Provider value="modal">
        <div className="wb-dialog-body">{children}</div>
      </WorkbenchDialogMode.Provider>
    </section>
  ) : (
    <ModalDialog title={title} onClose={onClose}>
      {children}
    </ModalDialog>
  );
}

function ModalDialog({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const previous = document.activeElement;
    const node = ref.current;
    node?.showModal();
    return () => {
      node?.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className="wb-dialog"
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <header>
        <h3>{title}</h3>
        <span className="grow" />
        <button
          type="button"
          className="icon-button"
          aria-label={"关闭" + title}
          onClick={onClose}
        >
          <X size={15} />
        </button>
      </header>
      <div className="wb-dialog-body">{children}</div>
    </dialog>
  );
}
