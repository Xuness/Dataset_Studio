import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, ErrorDetails } from "@studio/ui";
import { lakeKey } from "./queries.js";
import { sites } from "./model.js";

type Config = Schema["LakePipelineConfig"];
type Snapshot = Schema["LakePipelineSettings"];
type Numeric = Exclude<
  keyof Config,
  "version" | "sites" | "scan_mode" | "download_mib_per_second"
>;
type Field = [Numeric, string, number, number, string];
const resources: Field[] = [
  [
    "active_lakes",
    "同时更新数据湖数",
    1,
    3,
    "多个湖共享下面的编码、内存与暂存预算。",
  ],
  [
    "encode_concurrency",
    "编码并发数",
    1,
    16,
    "所有数据湖合计；大图还需满足解码内存预算。",
  ],
  [
    "decode_memory_mib",
    "解码内存预算（MiB）",
    64,
    1048576,
    "按像素和编码缓冲估算准入，不是进程内存的绝对上限。",
  ],
  [
    "spool_mib",
    "SSD 暂存预算（MiB）",
    64,
    1048576,
    "包含下载原件、编码产物与恢复收据；至少为单图下载上限的四倍。",
  ],
  [
    "reserve_mib",
    "磁盘保留空间（MiB）",
    0,
    1048576,
    "SSD 与图片归档盘低于保留空间时等待，不丢弃检查点。",
  ],
  [
    "max_download_mib",
    "单张下载上限（MiB）",
    1,
    4096,
    "限制收到的原文件大小；超限记录进入待检查。",
  ],
  [
    "max_image_pixels",
    "单张像素上限",
    1,
    1000000000,
    "宽 × 高；独立于保存策略中缩小后的尺寸。",
  ],
];
const batching: Field[] = [
  [
    "metadata_prefetch_records",
    "元数据前看记录数",
    200,
    1000000,
    "边扫描边下载时，队列达到此水位暂缓扫描，消费后继续；一页可能略超水位。",
  ],
  [
    "buffer_images",
    "每湖在途图片上限",
    1,
    512,
    "包括下载、等待编码、编码和待发布；同时受全局暂存字节预算约束。",
  ],
  [
    "publish_items",
    "发布批次记录数",
    1,
    512,
    "与体积、等待时间任一条件满足就发布；小批尾部会及时提交。",
  ],
  [
    "publish_mib",
    "发布批次体积（MiB）",
    1,
    65536,
    "按编码后的图片体积计算，达到阈值后提交完整图片。",
  ],
  [
    "publish_interval_seconds",
    "最长发布等待（秒）",
    0.1,
    300,
    "从第一张产物准备好开始计算；正在执行的元数据发布可能延后提交。",
  ],
];

export function PipelineSettings({ client }: { client: StudioClient }) {
  const key = [...lakeKey(client), "pipeline"];
  const cache = useQueryClient();
  const query = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => client.lakeUpdates.pipeline(signal),
    retry: false,
  });
  const [draft, setDraft] = useState<Snapshot | null>(null);
  const [site, setSite] = useState<keyof typeof sites>("yandere");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const snapshot = draft ?? query.data;
  if (!snapshot)
    return query.error ? (
      <ErrorDetails error={query.error} />
    ) : (
      <p>正在读取调度设置…</p>
    );
  const value = snapshot.value;
  const siteValue = value.sites[site] ?? snapshot.defaults.sites[site]!;
  function change(patch: Partial<Config>) {
    setDraft({ ...snapshot!, value: { ...value, ...patch } });
    setError(null);
    setNotice("");
  }
  function siteChange(patch: Partial<Schema["LakeSitePipeline"]>) {
    change({ sites: { ...value.sites, [site]: { ...siteValue, ...patch } } });
  }
  async function save() {
    setPending(true);
    setError(null);
    try {
      const saved = await client.lakeUpdates.savePipeline({
        expected_revision: snapshot!.revision,
        value,
      });
      cache.setQueryData(key, saved);
      setDraft(null);
      setNotice(
        "调度设置已保存。运行器会应用到后续工作；已开始的工作完成后释放占用，暂停任务仍需手动继续。",
      );
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  function fields(items: Field[]) {
    return items.map(([name, label, min, max, hint]) => (
      <div key={name} className="lake-parameter">
        <label>
          {label}
          <input
            aria-label={label}
            type="number"
            min={min}
            max={max}
            step={name === "publish_interval_seconds" ? "any" : 1}
            value={value[name]}
            onChange={(e) => change({ [name]: Number(e.target.value) })}
          />
        </label>
        <p className="lake-hint">{hint}</p>
      </div>
    ));
  }
  return (
    <div className="lake-pipeline-settings">
      <p className="lake-hint">
        这些设置跨项目、跨任务共享，可随时调整。保存后影响后续工作，不改变任务范围、图片保存配方或已经入湖的数据。
      </p>
      <fieldset disabled={pending}>
        <details open>
          <summary>站点网络</summary>
          <div className="lake-fields">
            <label>
              站点
              <select
                aria-label="调度站点"
                value={site}
                onChange={(e) => setSite(e.target.value as keyof typeof sites)}
              >
                {Object.entries(sites).map(([id, name]) => (
                  <option key={id} value={id}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              下载并发数
              <input
                aria-label="下载并发数"
                type="number"
                min={1}
                max={16}
                value={siteValue.download_concurrency}
                onChange={(e) =>
                  siteChange({ download_concurrency: Number(e.target.value) })
                }
              />
            </label>
            <label>
              API 请求/秒
              <input
                aria-label="API 请求/秒"
                type="number"
                min={0.05}
                max={10}
                step="any"
                value={siteValue.api_requests_per_second}
                onChange={(e) =>
                  siteChange({
                    api_requests_per_second: Number(e.target.value),
                  })
                }
              />
            </label>
            <label>
              图片请求/秒
              <input
                aria-label="图片请求/秒"
                type="number"
                min={0.05}
                max={50}
                step="any"
                value={siteValue.image_requests_per_second ?? ""}
                placeholder="留空不额外限频"
                onChange={(e) =>
                  siteChange({
                    image_requests_per_second:
                      e.target.value === "" ? null : Number(e.target.value),
                  })
                }
              />
            </label>
            <p className="lake-hint">
              请求/秒限制 HTTP
              请求的发起频率，包括重试；并发数限制同时下载的图片。图片/秒是处理结果，MiB/s
              是字节吞吐。429/503
              返回的冷却时间仍会被遵守。建议值用于起步实测，不代表站点公布的限额。
            </p>
          </div>
        </details>
        <details open>
          <summary>扫描与发布</summary>
          <div className="lake-fields">
            <label>
              扫描方式
              <select
                aria-label="扫描方式"
                value={value.scan_mode}
                onChange={(e) => change({ scan_mode: e.target.value })}
              >
                <option value="pipeline">边扫描边下载</option>
                <option value="metadata_first">先建立完整元数据清单</option>
              </select>
            </label>
            <p className="lake-hint">
              “先建立清单”仍受任务中的扫描预算限制；预算耗尽会暂停。元数据清单和下载原件的缓冲分别管理。
            </p>
            {fields(batching)}
          </div>
        </details>
        <details>
          <summary>全局资源与带宽</summary>
          <div className="lake-fields">
            {fields(resources)}
            <label>
              下载带宽上限（MiB/s）
              <input
                aria-label="下载带宽上限（MiB/s）"
                type="number"
                min={0.05}
                max={4096}
                step="any"
                value={value.download_mib_per_second ?? ""}
                placeholder="留空不限带宽"
                onChange={(e) =>
                  change({
                    download_mib_per_second:
                      e.target.value === "" ? null : Number(e.target.value),
                  })
                }
              />
            </label>
            <p className="lake-hint">
              带宽预算由所有湖共享，按收到的数据块节流，允许少量缓冲突发。下调资源限制不会强行终止正在执行的工作。
            </p>
          </div>
        </details>
      </fieldset>
      {error != null && <ErrorDetails error={error} />}
      {notice && <p role="status">{notice}</p>}
      <div className="lake-actions">
        <Button
          disabled={pending || !draft}
          className="primary"
          onClick={() => void save()}
        >
          保存调度设置
        </Button>
        <Button
          disabled={pending}
          onClick={() => {
            setDraft({
              ...snapshot,
              value: structuredClone(snapshot.defaults),
            });
            setNotice("");
          }}
        >
          填入建议值
        </Button>
        <Button
          disabled={pending}
          onClick={async () => {
            const result = await query.refetch();
            if (result.data) {
              setDraft(null);
              setError(null);
              setNotice("");
            }
          }}
        >
          重新读取设置
        </Button>
        <small>
          修订 {snapshot.revision}
          {draft ? " · 未保存" : ""}
        </small>
      </div>
    </div>
  );
}
