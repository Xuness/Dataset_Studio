import { useState } from "react";
import { Copy, Check, AlertCircle } from "lucide-react";

export function errorText(error: unknown) {
  if (error instanceof Error) {
    const code =
      "code" in error && typeof error.code === "string"
        ? error.code + ": "
        : "";
    return (
      code +
      error.message +
      ("requestId" in error && error.requestId
        ? "\n请求编号：" + String(error.requestId)
        : "")
    );
  }
  return String(error ?? "");
}
export function errorSummary(error: unknown) {
  const text = errorText(error);
  if (/SOURCE_RESOURCE_LIMIT|Out of Memory/.test(text))
    return "查询达到资源上限，请缩小范围或减少组合条件后重试。";
  if (/SOURCE_TIMEOUT/.test(text)) return "读取超时，可缩小查询范围后重试。";
  if (/REVISION_CONFLICT|选择.*修改|选择.*变化/.test(text))
    return "输入选择已变化，请按当前范围重新执行。";
  if (/SOURCE_CHANGED/.test(text)) return "来源已更新，请刷新范围后重试。";
  if (/SOURCE_BUSY/.test(text)) return "来源正在被其他操作占用，请稍后重试。";
  if (/SOURCE_UNAVAILABLE|METADATA_RUNTIME_UNAVAILABLE/.test(text))
    return "来源或读取环境暂不可用，请检查来源连接。";
  if (/RESULT_IN_USE|ARTIFACT_IN_USE/.test(text))
    return "仍有项目数据引用这项结果，暂时无法释放。";
  if (/SCOPE_PROJECT_MISMATCH/.test(text)) return "所选范围不属于当前项目。";
  if (/ENGINE_DISCONNECTED/.test(text))
    return "本机引擎连接中断，请重连后继续。";
  return (
    text
      .split("\n")[0]
      ?.replace(/^[A-Z_]+:\s*/, "")
      .slice(0, 220) || "操作未能完成。"
  );
}
export function CopyButton({
  text,
  label = "复制详情",
  className = "",
}: {
  text: string;
  label?: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState("");
  return (
    <span className={"copy-action " + className}>
      <button
        type="button"
        title={label}
        aria-label={label}
        onClick={() => {
          void navigator.clipboard
            .writeText(text)
            .then(() => {
              setCopied(true);
              setError("");
              setTimeout(() => setCopied(false), 2000);
            })
            .catch(() => setError("未能写入剪贴板，可展开详情手动复制。"));
        }}
      >
        {copied ? <Check size={13} /> : <Copy size={13} />}
        <span>{copied ? "已复制" : label}</span>
      </button>
      {error && <small role="status">{error}</small>}
    </span>
  );
}
export function ErrorDetails({
  error,
  title,
  compact = false,
}: {
  error: unknown;
  title?: string;
  compact?: boolean;
}) {
  const text = errorText(error);
  if (!text) return null;
  return (
    <div className={"error-details " + (compact ? "compact" : "")}>
      <div className="error-summary">
        <AlertCircle size={14} />
        <span>{title ?? errorSummary(error)}</span>
        <CopyButton text={text} />
      </div>
      <details>
        <summary>技术详情</summary>
        <pre tabIndex={0}>{text}</pre>
      </details>
    </div>
  );
}
