import { createContext, useContext, useEffect, useRef } from "react";
import { CheckCircle2, CircleAlert, Info, X } from "lucide-react";

export type Notice = {
  id: string;
  tone: "success" | "error" | "info";
  title: string;
  detail?: string;
  action?: { label: string; run: () => void };
};
export type Notify = (notice: Omit<Notice, "id"> & { id?: string }) => void;
const NotifyContext = createContext<Notify>(() => {});
/** Lets feature modules raise notices in the shell's notification stack. */
export const NotifyProvider = NotifyContext.Provider;
export function useNotify() {
  return useContext(NotifyContext);
}

function NoticeItem({
  notice,
  onDismiss,
}: {
  notice: Notice;
  onDismiss: (id: string) => void;
}) {
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  // Failures stay until dismissed; other notices fade unless hovered.
  const transient = notice.tone !== "error";
  function arm() {
    clearTimeout(timer.current);
    if (transient) timer.current = setTimeout(() => onDismiss(notice.id), 6000);
  }
  useEffect(() => {
    arm();
    return () => clearTimeout(timer.current);
    // Re-arm only when a different notice occupies this slot.
  }, [notice.id]);
  const Icon =
    notice.tone === "success"
      ? CheckCircle2
      : notice.tone === "error"
        ? CircleAlert
        : Info;
  return (
    <div
      className="wb-notice"
      data-tone={notice.tone}
      role={notice.tone === "error" ? "alert" : "status"}
      onPointerEnter={() => clearTimeout(timer.current)}
      onPointerLeave={arm}
    >
      <Icon size={16} className="wb-notice-icon" />
      <div className="wb-notice-body">
        <strong>{notice.title}</strong>
        {notice.detail && <span>{notice.detail}</span>}
        {notice.action && (
          <button
            type="button"
            className="wb-notice-action"
            onClick={() => {
              notice.action!.run();
              onDismiss(notice.id);
            }}
          >
            {notice.action.label}
          </button>
        )}
      </div>
      <button
        type="button"
        className="icon-button"
        aria-label="关闭通知"
        onClick={() => onDismiss(notice.id)}
      >
        <X size={13} />
      </button>
    </div>
  );
}

/** Editor-style notifications stacked above the status bar. */
export function NotificationStack({
  notices,
  onDismiss,
}: {
  notices: Notice[];
  onDismiss: (id: string) => void;
}) {
  if (!notices.length) return null;
  return (
    <div className="wb-notices" aria-live="polite">
      {notices.map((notice) => (
        <NoticeItem key={notice.id} notice={notice} onDismiss={onDismiss} />
      ))}
    </div>
  );
}
