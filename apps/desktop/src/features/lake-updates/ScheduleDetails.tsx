import { useState } from "react";
import { Button, ErrorDetails } from "@studio/ui";
import type { ApplicationModuleContext } from "@studio/ui";
import type { Schedule } from "./model.js";
import { rangeLabel, policyLabel } from "./model.js";
import { ImagePolicySummary } from "./ImagePolicySummary.js";
export function ScheduleDetails({
  client,
  schedule: s,
  refresh,
}: {
  client: ApplicationModuleContext["client"];
  schedule: Schedule;
  refresh: () => Promise<unknown>;
}) {
  const [hours, setHours] = useState(String((s.every_seconds ?? 86400) / 3600));
  const [periodic, setPeriodic] = useState(s.every_seconds != null),
    [enabled, setEnabled] = useState(s.enabled);
  const [when, setWhen] = useState(() => {
    const d = new Date(s.next_run_at);
    return new Date(d.getTime() - d.getTimezoneOffset() * 60000)
      .toISOString()
      .slice(0, 16);
  });
  const [revision, setRevision] = useState(s.revision),
    [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  async function save(remove = false) {
    setPending(true);
    setError(null);
    try {
      if (remove) {
        await client.lakeUpdates.removeSchedule(s.id, revision);
        setNotice("计划已移除");
      } else {
        const seconds = periodic ? Number(hours) * 3600 : null;
        if (
          seconds !== null &&
          (!Number.isSafeInteger(seconds) || seconds < 60 || seconds > 31622400)
        )
          throw new Error("间隔需为整数秒，范围 1 分钟至 366 天");
        const r = await client.lakeUpdates.saveSchedule({
          identity: s.id,
          revision,
          spec: s.definition,
          every_seconds: seconds,
          first_run_at: new Date(when).toISOString(),
          enabled,
        });
        setRevision(r.revision);
        setNotice("计划已保存");
      }
      await refresh();
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  return (
    <div className="lake-details">
      <details open>
        <summary>计划设置</summary>
        <p>{rangeLabel(s.definition)}</p>
        <p>{policyLabel(s.definition)}</p>
        <ImagePolicySummary policy={s.definition.media} />
        <div className="lake-fields">
          <label className="lake-check">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
            />
            启用计划
          </label>
          <label>
            下一次执行（本机时间）
            <input
              type="datetime-local"
              value={when}
              onChange={(e) => setWhen(e.target.value)}
            />
          </label>
          <label className="lake-check">
            <input
              type="checkbox"
              checked={periodic}
              onChange={(e) => setPeriodic(e.target.checked)}
            />
            按固定间隔重复
          </label>
          {periodic && (
            <label>
              间隔（小时）
              <input value={hours} onChange={(e) => setHours(e.target.value)} />
            </label>
          )}
          <p className="lake-hint">
            需要运行器保持运行。固定日期范围会重复执行同一范围；漏跑周期合并到最近一次。
          </p>
          <div className="lake-actions">
            <Button disabled={pending} onClick={() => void save()}>
              保存计划
            </Button>
            <Button
              disabled={pending}
              onClick={() => {
                if (window.confirm("移除此定时计划？已经创建的任务不受影响。"))
                  void save(true);
              }}
            >
              移除计划
            </Button>
          </div>
        </div>
        {error != null && <ErrorDetails error={error} />}
        {notice && <p role="status">{notice}</p>}
        <small>
          编辑修订 {revision} · 当前修订 {s.revision}
        </small>
      </details>
    </div>
  );
}
