import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, ErrorDetails } from "@studio/ui";
import { lakeKey, useLakeRefresh } from "./queries.js";

export function CollectionPipelineSettings({
  client,
}: {
  client: StudioClient;
}) {
  const query = useQuery({
    queryKey: [...lakeKey(client), "collection-pipeline"],
    queryFn: ({ signal }) => client.sourceCollections.pipeline(signal),
  });
  const [draft, setDraft] = useState<
      Schema["CollectionPipelineSettings"] | null
    >(null),
    [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  const refresh = useLakeRefresh(client),
    snapshot = draft ?? query.data;
  if (!snapshot)
    return query.error ? (
      <ErrorDetails error={query.error} />
    ) : (
      <p>正在读取 Pixiv 调度设置…</p>
    );
  const value = snapshot.value;
  function change(patch: Partial<typeof value>) {
    setDraft({ ...snapshot!, value: { ...value, ...patch } });
    setNotice("");
  }
  async function save() {
    setPending(true);
    setError(null);
    try {
      await client.sourceCollections.savePipeline({
        expected_revision: snapshot!.revision,
        value,
      });
      setDraft(null);
      setNotice("Pixiv 调度参数已保存，后续时间片生效。");
      await refresh();
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  return (
    <details open>
      <summary>Pixiv 采集网络与队列</summary>
      <fieldset disabled={pending}>
        <div className="lake-fields">
          <label>
            Pixiv 下载并发
            <input
              type="number"
              min={1}
              max={16}
              value={value.pixiv.download_concurrency}
              onChange={(e) =>
                change({
                  pixiv: {
                    ...value.pixiv,
                    download_concurrency: Number(e.target.value),
                  },
                })
              }
            />
          </label>
          <label>
            Pixiv API 请求 / 秒
            <input
              type="number"
              min={0.05}
              max={10}
              step="any"
              value={value.pixiv.api_requests_per_second}
              onChange={(e) =>
                change({
                  pixiv: {
                    ...value.pixiv,
                    api_requests_per_second: Number(e.target.value),
                  },
                })
              }
            />
          </label>
          <label>
            Pixiv 图片请求 / 秒
            <input
              type="number"
              min={0.05}
              max={50}
              step="any"
              placeholder="空白为不设频率上限"
              value={value.pixiv.image_requests_per_second ?? ""}
              onChange={(e) =>
                change({
                  pixiv: {
                    ...value.pixiv,
                    image_requests_per_second: e.target.value
                      ? Number(e.target.value)
                      : null,
                  },
                })
              }
            />
          </label>
          <label>
            Pixiv 待下载队列上限
            <input
              type="number"
              min={1}
              max={100000}
              value={value.pending_media_limit}
              onChange={(e) =>
                change({ pending_media_limit: Number(e.target.value) })
              }
            />
          </label>
          <label>
            Pixiv 发布积压上限（MiB）
            <input
              type="number"
              min={16}
              max={256}
              value={value.publication_backlog_mib}
              onChange={(e) =>
                change({ publication_backlog_mib: Number(e.target.value) })
              }
            />
          </label>
          <label>
            Pixiv 调度时间片（秒）
            <input
              type="number"
              min={1}
              max={300}
              value={value.time_slice_seconds}
              onChange={(e) =>
                change({ time_slice_seconds: Number(e.target.value) })
              }
            />
          </label>
        </div>
      </fieldset>
      <p className="lake-hint">
        与其他数据湖共享编码、解码内存、带宽和暂存预算。每个 Pixiv
        湖的元数据按单路处理。
      </p>
      <Button disabled={pending || !draft} onClick={() => void save()}>
        保存 Pixiv 调度
      </Button>
      {error != null && <ErrorDetails error={error} />}
      {notice && <p role="status">{notice}</p>}
    </details>
  );
}
