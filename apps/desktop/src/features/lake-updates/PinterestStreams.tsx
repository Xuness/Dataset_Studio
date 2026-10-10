import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import { Button, ErrorDetails } from "@studio/ui";
import { lakeKey } from "./queries.js";
import { bytesLabel } from "./model.js";
import {
  pinterestActive,
  pinterestReason,
  pinterestStates,
  pinterestTaskLabels,
  type PinterestJob,
} from "./pinterestModel.js";

export function PinterestStreams({
  client,
  job,
}: {
  client: StudioClient;
  job: PinterestJob;
}) {
  const [cursors, setCursors] = useState<string[]>([""]);
  const streams = useQuery({
    queryKey: [...lakeKey(client), "pinterest-streams", job.id, cursors.at(-1)],
    queryFn: ({ signal }) =>
      client.pinterestCollections.streams(job.id, {
        limit: 15,
        cursor: cursors.at(-1) || undefined,
        signal,
      }),
    refetchInterval: pinterestActive(job) ? 4000 : false,
  });
  const origins = Object.keys(pinterestTaskLabels).filter(
    (k) => job.metrics[`${k}:requests`] || job.metrics[`${k}:pages`],
  );
  return (
    <>
      <details open>
        <summary>发现流与覆盖依据</summary>
        {streams.error && <ErrorDetails error={streams.error} />}
        {streams.data?.items.map((stream) => (
          <div className="collection-task" key={stream.scan_id}>
            <strong>
              {pinterestTaskLabels[stream.entrypoint] ?? stream.entrypoint} ·{" "}
              {pinterestStates[stream.state] ?? stream.state}
            </strong>
            <small>{stream.subject_id}</small>
            <p>
              {stream.pages} 页 ·{" "}
              {stream.entrypoint === "board_sections"
                ? stream.total == null
                  ? "分区总量未知"
                  : `来源报告 ${stream.total} 个分区`
                : `${stream.unique_pins} 个不同 Pin · ${stream.members} 次出现 · ${stream.total == null ? "来源总量未知" : `来源报告 ${stream.total} 个 Pin`}`}
              {stream.has_cursor ? " · 保留后续游标" : ""}
            </p>
            <p>
              已核对 {stream.samples_checked} 份列表清单
              {stream.force_detail ? " · 已切换为详情确认" : ""}
            </p>
            {stream.reason && <p>{pinterestReason(stream.reason)}</p>}
          </div>
        ))}
        {!streams.isPending && !streams.data?.items.length && (
          <p className="lake-hint">此任务没有分页发现流。</p>
        )}
        {(cursors.length > 1 || streams.data?.next_cursor) && (
          <footer className="lake-pagination">
            <Button
              disabled={cursors.length === 1 || streams.isFetching}
              onClick={() => setCursors((v) => v.slice(0, -1))}
            >
              上一页发现流
            </Button>
            <span>第 {cursors.length} 页</span>
            <Button
              disabled={!streams.data?.next_cursor || streams.isFetching}
              onClick={() =>
                setCursors((v) => [...v, streams.data!.next_cursor!])
              }
            >
              下一页发现流
            </Button>
          </footer>
        )}
        <p className="lake-hint">
          来源总量是图版或分区信息中的计数，可能随时间变化。结束标记只描述本次访问下的流；可疑结束页会复查，已观察数量仍不足时保留缺口。
        </p>
      </details>
      {origins.length > 0 && (
        <details>
          <summary>入口成本与结果</summary>
          <div className="lake-fields">
            {origins.map((kind) => (
              <div key={kind}>
                <strong>{pinterestTaskLabels[kind]}</strong>
                <p>
                  {job.metrics[`${kind}:requests`] ?? 0} 次请求 ·{" "}
                  {job.metrics[`${kind}:detail_requests`] ?? 0} 次关联详情 ·{" "}
                  {job.metrics[`${kind}:unique_pins`] ?? 0} 个观察 Pin ·{" "}
                  {job.metrics[`${kind}:admitted_pins`] ?? 0} 个首次准入 Pin
                </p>
                <p>
                  {job.metrics[`${kind}:unique_objects`] ?? 0} 个文件对象 · 下载{" "}
                  {bytesLabel(job.metrics[`${kind}:download_bytes`] ?? 0)}
                </p>
              </div>
            ))}
            <p className="lake-hint">
              同一 Pin
              或文件可能出现在多个入口，各入口的结果数不相加作为全湖新增数。
            </p>
          </div>
        </details>
      )}
    </>
  );
}
