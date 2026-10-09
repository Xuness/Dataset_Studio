import { useState } from "react";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, ErrorDetails } from "@studio/ui";
import { pinterestRange } from "./pinterestModel.js";
import { dateLabel } from "./model.js";
import { useLakeRefresh } from "./queries.js";

export function PinterestScheduleDetails({
  client,
  schedule: s,
  onJob,
}: {
  client: StudioClient;
  schedule: Schema["PinterestSchedule"];
  onJob: (id: string) => void;
}) {
  const refresh = useLakeRefresh(client);
  const [hours, setHours] = useState(s.every_seconds / 3600),
    [enabled, setEnabled] = useState(s.enabled);
  const [revision, setRevision] = useState(s.revision),
    [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  const [command, setCommand] = useState<{
    remove: boolean;
    key: string;
    save?: Schema["SavePinterestSchedule"];
  } | null>(null);
  async function submit(remove: boolean) {
    setPending(true);
    setError(null);
    try {
      if (
        !remove &&
        (!Number.isSafeInteger(hours * 3600) || hours < 1 / 60 || hours > 8784)
      )
        throw new Error("复查间隔需为 1 分钟至 366 天。");
      const frozen = command ?? {
        remove,
        key: crypto.randomUUID(),
        ...(!remove
          ? {
              save: {
                id: s.id,
                request_key: crypto.randomUUID(),
                expected_revision: revision,
                definition: s.definition,
                every_seconds: hours * 3600,
                first_run_at: s.next_run_at,
                enabled,
              },
            }
          : {}),
      };
      setCommand(frozen);
      if (frozen.remove) {
        await client.pinterestCollections.removeSchedule(s.id, {
          request_key: frozen.key,
          expected_revision: revision,
        });
        setNotice("计划已移除，历史任务仍保留。");
      } else {
        const result = await client.pinterestCollections.saveSchedule(
          frozen.save!,
        );
        setRevision(result.revision);
        setNotice("计划已保存。");
      }
      setCommand(null);
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
        <summary>Pinterest 周期复查</summary>
        <p>{pinterestRange(s.definition)}</p>
        <fieldset className="lake-fields" disabled={pending || !!command}>
          <label className="lake-check">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
            />
            启用计划
          </label>
          <label>
            复查间隔（小时）
            <input
              type="number"
              min={1 / 60}
              max={8784}
              step="any"
              value={hours}
              onChange={(e) => setHours(Number(e.target.value))}
            />
          </label>
        </fieldset>
        <p>下次执行：{dateLabel(s.next_run_at)}</p>
        <p className="lake-hint">
          同一湖有暂停、预算用完或待检查任务时会等待。每轮重新观察来源，历史未再次出现的成员仍保留。
        </p>
        <div className="lake-actions">
          <Button
            disabled={pending || command?.remove === true}
            onClick={() => void submit(false)}
          >
            {command && !command.remove ? "重试保存" : "保存计划"}
          </Button>
          <Button
            disabled={pending || command?.remove === false}
            onClick={() => void submit(true)}
          >
            {command?.remove ? "重试移除" : "移除计划"}
          </Button>
          {s.last_job && (
            <Button onClick={() => onJob(s.last_job!)}>查看上一轮任务</Button>
          )}
        </div>
        {error != null && <ErrorDetails error={error} />}
        {notice && <p role="status">{notice}</p>}
      </details>
    </div>
  );
}
