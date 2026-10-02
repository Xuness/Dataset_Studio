import { useState } from "react";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, ErrorDetails } from "@studio/ui";
import { collectionRange } from "./collectionModel.js";
import { dateLabel } from "./model.js";
import { useLakeRefresh } from "./queries.js";

export function CollectionScheduleDetails({
  client,
  schedule: s,
  onJob,
}: {
  client: StudioClient;
  schedule: Schema["CollectionSchedule"];
  onJob: (id: string) => void;
}) {
  const refresh = useLakeRefresh(client);
  const [hours, setHours] = useState(s.every_seconds / 3600),
    [enabled, setEnabled] = useState(s.enabled);
  const [revision, setRevision] = useState(s.revision),
    [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  async function save(remove = false) {
    setPending(true);
    setError(null);
    try {
      if (remove) {
        await client.sourceCollections.removeSchedule(s.id, {
          request_key: crypto.randomUUID(),
          expected_revision: revision,
        });
        setNotice("计划已移除");
      } else {
        if (
          !Number.isSafeInteger(hours * 3600) ||
          hours < 1 / 60 ||
          hours > 8784
        )
          throw new Error("间隔需为 1 分钟至 366 天。");
        const result = await client.sourceCollections.saveSchedule({
          id: s.id,
          request_key: crypto.randomUUID(),
          expected_revision: revision,
          definition: s.definition,
          every_seconds: hours * 3600,
          first_run_at: s.next_run_at,
          enabled,
        });
        setRevision(result.revision);
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
        <summary>Pixiv 周期复查</summary>
        <p>{collectionRange(s.definition)}</p>
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
            间隔（小时）
            <input
              type="number"
              min={1 / 60}
              max={8784}
              step="any"
              value={hours}
              onChange={(e) => setHours(Number(e.target.value))}
            />
          </label>
          <p>下次执行：{dateLabel(s.next_run_at)}</p>
          <p className="lake-hint">
            上一轮暂停、预算用完或等待凭据时，计划会等待该轮结束。漏跑周期合并成一次最新快照。
          </p>
          <div className="lake-actions">
            <Button disabled={pending} onClick={() => void save()}>
              保存计划
            </Button>
            <Button disabled={pending} onClick={() => void save(true)}>
              移除计划
            </Button>
            {s.last_job && (
              <Button onClick={() => onJob(s.last_job!)}>查看上一轮任务</Button>
            )}
          </div>
        </div>
        {error != null && <ErrorDetails error={error} />}
        {notice && <p role="status">{notice}</p>}
      </details>
    </div>
  );
}
