import { pinterestEntrypoints, type PinterestDraft } from "./pinterestModel.js";

export function PinterestOptions({
  d,
  change,
}: {
  d: PinterestDraft;
  change: (patch: Partial<PinterestDraft>) => void;
}) {
  return (
    <>
      <details open>
        <summary>列表清单与元数据</summary>
        <div className="lake-fields">
          <label>
            详情补取
            <select
              aria-label="详情补取"
              value={d.metadataPolicy}
              onChange={(e) =>
                change({
                  metadataPolicy: e.target
                    .value as PinterestDraft["metadataPolicy"],
                })
              }
            >
              <option value="sample">抽样核对列表清单</option>
              <option value="all">补取每个准入 Pin 的元数据</option>
              <option value="none">只取形成清单所需的详情</option>
            </select>
          </label>
          {d.metadataPolicy === "sample" && (
            <label>
              每个发现流的抽样数
              <input
                type="number"
                min={1}
                max={20}
                value={d.sampleSize}
                onChange={(e) => change({ sampleSize: Number(e.target.value) })}
              />
            </label>
          )}
          <p className="lake-hint">
            列表字段足够时直接取得原图。补取使用单独的详情预算；列表与抽样详情不一致时，后续条目改为详情确认。
            {d.metadataPolicy === "none" && "当前不会抽样核对列表清单。"}
          </p>
        </div>
      </details>
      <details>
        <summary>发现扩展与积压限制</summary>
        <div className="lake-fields">
          <label className="lake-check">
            <input
              type="checkbox"
              checked={d.includeSections}
              onChange={(e) => change({ includeSections: e.target.checked })}
            />
            同时扫描图版分区
          </label>
          {Object.entries(pinterestEntrypoints).map(([key, label]) => (
            <label className="lake-check" key={key}>
              <input
                type="checkbox"
                checked={d.entrypoints.includes(key)}
                onChange={(e) =>
                  change({
                    entrypoints: e.target.checked
                      ? [...d.entrypoints, key]
                      : d.entrypoints.filter((v) => v !== key),
                  })
                }
              />
              {label}
            </label>
          ))}
          <label>
            最大扩展深度
            <input
              type="number"
              min={1}
              max={3}
              value={d.depth}
              onChange={(e) => change({ depth: Number(e.target.value) })}
            />
          </label>
          <label>
            每类发现入口的请求上限
            <input
              type="number"
              min={0}
              max={10000}
              value={d.entryRequests}
              onChange={(e) =>
                change({ entryRequests: Number(e.target.value) })
              }
            />
          </label>
          <label>
            暂停发现的积压阈值
            <input
              type="number"
              min={1}
              max={1000}
              value={d.maxPending}
              onChange={(e) => change({ maxPending: Number(e.target.value) })}
            />
          </label>
          <p className="lake-hint">
            推荐和搜索保存独立快照。图版成员、推荐结果与主题关联会分别记录；流的总量未知时不会显示虚假的完成百分比。
          </p>
        </div>
      </details>
      <details>
        <summary>历史文件复用</summary>
        <div className="lake-fields">
          <label>
            复用策略
            <select
              value={d.reuseMode}
              onChange={(e) =>
                change({
                  reuseMode: e.target.value as PinterestDraft["reuseMode"],
                })
              }
            >
              <option value="none">每轮重新获取原文件</option>
              <option value="revalidate">先向 CDN 核验，未改变时复用</option>
              <option value="historical">直接沿用期限内的已存文件</option>
            </select>
          </label>
          {d.reuseMode !== "none" && (
            <label>
              有效期限（小时）
              <input
                type="number"
                min={1}
                max={8760}
                value={d.reuseHours}
                onChange={(e) => change({ reuseHours: Number(e.target.value) })}
              />
            </label>
          )}
          <p className="lake-hint">
            只在当前湖内核对媒体、访问条件与保存方式后复用。历史复用保留上次核验时间；每轮仍重新观察来源元数据。
          </p>
        </div>
      </details>
      <details>
        <summary>周期复查</summary>
        <div className="lake-fields">
          <label className="lake-check">
            <input
              type="checkbox"
              checked={d.periodic}
              onChange={(e) => change({ periodic: e.target.checked })}
            />
            建立周期计划
          </label>
          {d.periodic && (
            <>
              <label>
                复查间隔（小时）
                <input
                  type="number"
                  min={1 / 60}
                  max={8784}
                  step="any"
                  value={d.intervalHours}
                  onChange={(e) =>
                    change({ intervalHours: Number(e.target.value) })
                  }
                />
              </label>
              <label className="lake-check">
                <input
                  type="checkbox"
                  checked={d.scheduleEnabled}
                  onChange={(e) =>
                    change({ scheduleEnabled: e.target.checked })
                  }
                />
                创建后启用计划
              </label>
            </>
          )}
          <p className="lake-hint">
            启用后先执行一轮，再按间隔复查。同一湖有未结束任务时会等待，漏跑的周期合并为一次最新观察。
          </p>
        </div>
      </details>
    </>
  );
}
