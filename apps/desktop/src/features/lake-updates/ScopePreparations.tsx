import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, DraftStatus, ErrorDetails } from "@studio/ui";
import type { ApplicationModuleContext } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import { lakeKey, useLakePreference, useLakeRefresh } from "./queries.js";
import { decodeDraft, initialDraft, lakeLabel } from "./model.js";
import type { Lake } from "./model.js";
type Submission = {
  request_key: string;
  scope: Schema["ScopeRef"] | null;
  label?: string;
};
const initial: Submission = { request_key: "", scope: null };
function decode(v: unknown): Submission | null {
  if (!v || typeof v !== "object") return null;
  const s = v as Submission;
  return typeof s.request_key === "string" &&
    (s.scope === null || typeof s.scope?.project_id === "string")
    ? s
    : null;
}
export function ScopePreparations({
  client,
  project,
  lakes,
  onUse,
}: {
  client: ApplicationModuleContext["client"];
  project: ApplicationModuleContext["project"];
  lakes: Lake[];
  onUse: () => void;
}) {
  const refresh = useLakeRefresh(client),
    submission = useLakePreference<Submission>(
      client,
      "studio.lake-updates.scope-submission",
      initial,
      decode,
    );
  const form = useLakePreference(
    client,
    "studio.lake-updates.composer",
    initialDraft,
    decodeDraft,
  );
  const [selection, setSelection] = useState(""),
    [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null);
  const rows = useQuery({
    queryKey: [...lakeKey(client), "preparations"],
    queryFn: ({ signal }) => client.lakeUpdates.preparations(signal),
    refetchInterval: 3000,
  });
  const labels: Record<string, string> = {
    preparing: "正在固定和映射成员",
    needs_review: "需要检查",
    cancelling: "正在取消",
    cancelled: "已取消",
    ready: "范围已就绪",
  };
  async function submit() {
    setPending(true);
    setError(null);
    try {
      let request = submission.value;
      if (!request.scope) {
        const option = project?.inputs.find((i) => i.value === selection);
        if (!option) throw new Error("请选择项目范围");
        request = {
          request_key: crypto.randomUUID(),
          scope: option.scope,
          label: option.label,
        };
        submission.controller.set(request);
        await submission.controller.flush();
      }
      await client.lakeUpdates.prepareScope({
        request_key: request.request_key,
        scope: request.scope!,
        label: request.label ?? null,
      });
      submission.controller.set(initial);
      await submission.controller.flush();
      await refresh();
    } catch (e) {
      setError(e);
      await refresh();
    } finally {
      setPending(false);
    }
  }
  async function use(row: Schema["LakeUpdatePreparation"]) {
    setError(null);
    try {
      const d = form.value;
      if (d.submissions.some((s) => !s.jobId))
        throw new Error(
          "上一批更新仍有未确认的提交，请先在新建更新中完成确认。",
        );
      const inputs = row.inputs.filter((i) => i.count > 0);
      if (!inputs.length) throw new Error("此范围没有可更新帖子");
      form.controller.set({
        ...d,
        kind: "input",
        lakes: inputs.map((i) => i.library_id),
        inputIds: Object.fromEntries(
          inputs.map((i) => [i.library_id, i.input_id]),
        ),
        execution: "now",
        submissions: [],
      });
      await form.controller.flush();
      onUse();
    } catch (e) {
      setError(e);
    }
  }
  return (
    <div className="lake-preparations">
      <div className="lake-fields">
        <strong>
          {project ? `来自项目：${project.name}` : "已保存的项目范围"}
        </strong>
        <p className="lake-hint">
          固定成员后在后台映射全部关联帖子。这里只准备范围，不会发起源站抓取；关闭项目后仍可继续。
        </p>
        <DraftStatus controller={submission.controller} quiet />
        {project && (
          <label>
            选择范围
            <select
              aria-label="选择范围"
              disabled={!!submission.value.scope}
              value={selection}
              onChange={(e) => setSelection(e.target.value)}
            >
              <option value="">请选择工作集、选择或查询结果</option>
              {project.inputs.map((i) => (
                <option key={i.value} value={i.value} disabled={i.count === 0}>
                  {i.label}
                  {i.count == null ? "" : ` · ${i.count.toLocaleString()} 张`}
                </option>
              ))}
            </select>
          </label>
        )}
        {(project || submission.value.scope) && (
          <Button
            disabled={
              pending ||
              !submission.editable ||
              (!selection && !submission.value.scope)
            }
            onClick={() => void submit()}
          >
            {submission.value.scope
              ? "确认上次范围准备请求"
              : "在后台准备此范围"}
          </Button>
        )}
        {error != null && <ErrorDetails error={error} />}
        {rows.error && <ErrorDetails error={rows.error} />}
      </div>
      <div className="lake-table-scroll">
        <table className="lake-table">
          <thead>
            <tr>
              <th>范围准备（最近 100 条）</th>
              <th>已映射图片</th>
              <th>帖子输入</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {rows.data?.items.map((row) => (
              <tr key={row.id}>
                <td>
                  <strong>{row.label || "固定图片范围"}</strong>
                  <small>
                    {row.project_name || "项目范围"} ·{" "}
                    {labels[row.state] ?? row.state}
                  </small>
                  {row.error && <p>{row.error}</p>}
                </td>
                <td>
                  {row.processed.toLocaleString()} /{" "}
                  {row.total?.toLocaleString() ?? "尚未知"}
                </td>
                <td>
                  {row.inputs.map((i) => (
                    <div key={i.library_id}>
                      {lakeLabel(lakes, i.library_id)} ·{" "}
                      {i.count.toLocaleString()} 帖
                    </div>
                  ))}
                </td>
                <td>
                  {row.state === "ready" && (
                    <Button
                      disabled={
                        !form.editable ||
                        row.inputs.some(
                          (i) => !lakes.some((l) => l.id === i.library_id),
                        )
                      }
                      onClick={() => void use(row)}
                    >
                      使用此范围新建更新
                    </Button>
                  )}
                  {["preparing", "needs_review"].includes(row.state) && (
                    <Button
                      disabled={pending}
                      onClick={() => {
                        setPending(true);
                        void client.lakeUpdates
                          .preparationAction(row.id, "cancel")
                          .then(refresh)
                          .catch(setError)
                          .finally(() => setPending(false));
                      }}
                    >
                      取消准备
                    </Button>
                  )}
                  {row.state === "needs_review" && (
                    <Button
                      disabled={pending}
                      onClick={() => {
                        setPending(true);
                        void client.lakeUpdates
                          .preparationAction(row.id, "resume")
                          .then(refresh)
                          .catch(setError)
                          .finally(() => setPending(false));
                      }}
                    >
                      继续准备
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
