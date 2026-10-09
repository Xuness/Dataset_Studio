import { displayTag } from "@studio/ui";
import { RotateCw, FileJson2 } from "lucide-react";
import { Button, ErrorDetails, ratingLabel } from "@studio/ui";
import { StudioError } from "@studio/client";
import type { StudioClient } from "@studio/client";
import type { Asset, MetadataField, MetadataValue } from "@studio/contracts";
import { useMetadata } from "./useMetadata.js";
import { useQuery } from "@tanstack/react-query";
import { RankingInputEvidence } from "../ranking/RankingInputEvidence.js";
import { PixivOrigin } from "./PixivOrigin.js";

const labels: Record<string, string> = {
  normalization_issues: "规范化说明",
  rating: "分级",
  tags: "标签",
  "tags.general": "一般",
  "tags.artist": "画师",
  "tags.character": "角色",
  "tags.copyright": "作品",
  "tags.meta": "元标签",
  source_url: "来源地址",
  source_width: "来源宽度",
  source_height: "来源高度",
  source_bytes: "来源字节数",
  source_extension: "来源格式",
  source_md5: "来源 MD5",
  created_at: "创建时间",
  updated_at: "更新时间",
  "danbooru.score": "评分",
  "danbooru.fav_count": "收藏数",
  "danbooru.uploader_id": "上传者 ID",
  "danbooru.parent_id": "父条目 ID",
  "danbooru.pixiv_id": "Pixiv ID",
  "danbooru.is_deleted": "已删除",
  "danbooru.is_banned": "已屏蔽",
  "danbooru.is_pending": "待审核",
  "danbooru.is_flagged": "已标记",
  "demo.sample_number": "生成样本编号",
  title: "作品标题",
  caption_html: "作品说明原文",
  author_id: "作者 ID",
  source_variant: "来源版本",
  "pixiv.x_restrict": "Pixiv 分级标记",
  "pixiv.ai_type": "Pixiv AI 标记",
  "pixiv.bookmark_count": "收藏数",
  "pixiv.view_count": "浏览数",
  "pixiv.like_count": "点赞数",
};
const quality: Record<string, string> = {
  date_only: "仅日期",
  exact: "精确时间",
  unknown: "精度未知",
  not_observed: "生成示例，无观察时间",
  captured_at: "采集时间",
};
function Failure({ error, refresh }: { error: Error; refresh: () => void }) {
  const code = error instanceof StudioError ? error.code : "ERROR";
  const title: Record<string, string> = {
    SOURCE_BUSY: "来源忙碌",
    SOURCE_CHANGED: "读取版本已变化",
    SOURCE_UNAVAILABLE: "来源不可用",
    SOURCE_TIMEOUT: "读取超时",
    SOURCE_RESOURCE_LIMIT: "读取达到资源上限",
    NOT_FOUND: "关联记录不存在",
    METADATA_UNSUPPORTED: "来源尚不支持元数据",
    METADATA_RUNTIME_UNAVAILABLE: "元数据运行库未就绪",
  };
  return (
    <div className="metadata-message metadata-error" role="alert">
      <ErrorDetails error={error} title={title[code] ?? "元数据读取失败"} />
      <Button onClick={refresh}>重新读取对象</Button>
    </div>
  );
}
function Value({
  value,
  onTag,
}: {
  value: MetadataValue | null | undefined;
  onTag?: ((tag: string) => void) | undefined;
}) {
  if (!value) return <span className="metadata-unknown">未记录</span>;
  if (value.type === "boolean") return <>{value.value ? "是" : "否"}</>;
  if (value.type === "tags")
    return value.value.length ? (
      <div className="metadata-tags">
        {value.value.map((tag, i) => (
          <button
            type="button"
            key={i}
            disabled={!onTag}
            title="用完整标签创建查询草稿"
            onClick={() => onTag?.(tag)}
          >
            {displayTag(tag)}
          </button>
        ))}
      </div>
    ) : (
      <span className="metadata-unknown">空标签</span>
    );
  return <>{value.value || <span className="metadata-unknown">空值</span>}</>;
}
function Field({
  field,
  onTag,
}: {
  field: MetadataField;
  onTag?: ((tag: string) => void) | undefined;
}) {
  return (
    <div className="metadata-field" title={field.provenance}>
      <dt>
        {field.label ??
          labels[field.name] ??
          labels["danbooru." + field.name.split(".").at(-1)] ??
          field.name}
      </dt>
      <dd>
        {field.name === "rating" && field.value?.type === "text" ? (
          <span title="使用数据湖已保存的统一分级；来源原值可在原始元数据中查看。">
            {ratingLabel(field.value.value)}
          </span>
        ) : !field.value &&
          ["explicit_null", "not_in_response"].includes(
            field.missing_reason ?? "",
          ) ? (
          <span className="metadata-unknown">
            {field.missing_reason === "explicit_null"
              ? "来源明确返回空值"
              : "本次响应未包含此字段"}
          </span>
        ) : !field.value &&
          field.missing_reason?.startsWith("normalization_issue:") ? (
          <span
            className="metadata-unknown"
            title={field.missing_reason.slice("normalization_issue:".length)}
          >
            未能归一化，详见规范化说明
          </span>
        ) : (
          <Value
            value={field.value}
            onTag={field.truncated ? undefined : onTag}
          />
        )}
        {field.truncated && (
          <small className="metadata-unknown">字段过长，显示前 8192 字符</small>
        )}
      </dd>
    </div>
  );
}
// Keep source numbers and escape sequences exact, including integers beyond JS precision.
function rawText(text: string) {
  return text;
}

export function MetadataInspector({
  client,
  projectId,
  asset,
  onFilterTag,
  onFilterOrigin,
}: {
  client: StudioClient;
  projectId: string;
  asset: Asset;
  onFilterTag?: (tag: string) => void;
  onFilterOrigin?: (field: "work.id" | "author.id", value: string) => void;
}) {
  const ranking = asset.ranking;
  const snapshot = useQuery({
    queryKey: [
      "project",
      projectId,
      "ranking-row",
      ranking?.artifact_id,
      ranking?.ordinal,
    ],
    queryFn: ({ signal }) =>
      client.ranking.row(
        projectId,
        ranking!.artifact_id,
        ranking!.ordinal,
        signal,
      ),
    enabled: !!ranking?.record_id,
    staleTime: Infinity,
  });
  const fixed = snapshot.data?.input;
  const matches =
    fixed?.source_id === asset.key.source_id &&
    fixed?.asset_id === asset.key.asset_id;
  const state = useMetadata(
    client,
    projectId,
    asset.key,
    matches ? fixed : undefined,
    asset.summary?.version ?? undefined,
  );
  const { overview, observations, raw, data, record, observation } = state;
  return (
    <section className="metadata-inspector" aria-label="元数据检查">
      <div className="metadata-heading">
        <h3>元数据</h3>
        <button
          className="icon-button"
          title="重新读取元数据"
          aria-label="重新读取元数据"
          onClick={state.refresh}
        >
          <RotateCw size={13} />
        </button>
      </div>
      {ranking?.record_id && snapshot.isPending && (
        <p role="status">正在读取评分依据…</p>
      )}
      {ranking && snapshot.error && (
        <Failure
          error={snapshot.error}
          refresh={() => void snapshot.refetch()}
        />
      )}
      {matches && fixed && <RankingInputEvidence input={fixed} />}
      {overview.isPending && (
        <p className="metadata-message" role="status">
          正在读取对象关联…
        </p>
      )}
      {overview.error && (
        <Failure error={overview.error} refresh={state.refresh} />
      )}
      {data && (
        <>
          {record?.work_id && (
            <PixivOrigin
              client={client}
              projectId={projectId}
              sourceId={asset.key.source_id}
              version={data.version.token}
              workId={record.work_id}
              ordinal={record.ordinal}
              kind={record.kind}
              representation={record.representation}
              onFilter={onFilterOrigin}
            />
          )}
          {record?.pin_origin && (
            <section
              className="metadata-primary"
              aria-label="Pinterest Pin 资料"
            >
              <h4>Pin {record.pin_origin.pin_id}</h4>
              <p>原始图片</p>
              <a
                href={`https://www.pinterest.com/pin/${encodeURIComponent(record.pin_origin.pin_id)}/`}
                target="_blank"
                rel="noreferrer"
              >
                在 Pinterest 中打开
              </a>
            </section>
          )}
          {observation && (
            <div className="metadata-primary">
              <p className="metadata-basis">
                {observation.relation === "asset_origin"
                  ? "来源初始记录"
                  : "所选历史记录"}{" "}
                · {observation.observed_at?.slice(0, 10) ?? "时间未知"}
              </p>
              <dl className="metadata-fields">
                {observation.fields
                  .filter((f) =>
                    [
                      "rating",
                      "tags",
                      "source_width",
                      "source_height",
                    ].includes(f.name),
                  )
                  .map((f) => (
                    <Field key={f.name} field={f} onTag={onFilterTag} />
                  ))}
              </dl>
            </div>
          )}
          {!data.records.length && (
            <p className="metadata-message">这张图片没有可用的来源记录。</p>
          )}
          <details
            className="metadata-details metadata-connections"
            open={data.records.length > 1}
          >
            <summary>
              来源与历史记录{record?.post_id ? " · #" + record.post_id : ""}
            </summary>

            <dl className="metadata-fields">
              <div className="metadata-field">
                <dt>存储尺寸</dt>
                <dd>
                  {data.stored_width && data.stored_height ? (
                    `${data.stored_width} × ${data.stored_height}`
                  ) : (
                    <span className="metadata-unknown">尚未检查图片</span>
                  )}
                </dd>
              </div>
            </dl>
            {!data.records.length && (
              <p className="metadata-message">
                这个存储对象没有关联的来源记录。
              </p>
            )}
            {!!data.records.length && (
              <>
                <label className="metadata-label">
                  来源记录
                  <select
                    aria-label="来源记录"
                    value={record?.record_id ?? ""}
                    onChange={(e) => state.selectRecord(e.target.value)}
                  >
                    {record &&
                      !data.records.some(
                        (r) => r.record_id === record.record_id,
                      ) && (
                        <option value={record.record_id}>
                          评分记录 #{record.post_id ?? "未知"}
                        </option>
                      )}
                    {data.records.map((r) => (
                      <option key={r.record_id} value={r.record_id}>
                        {r.pin_origin
                          ? `Pin ${r.pin_origin.pin_id} · 原图`
                          : r.work_id
                            ? `作品 ${r.work_id} · 第 ${(r.ordinal ?? 0) + 1} 页`
                            : r.post_id
                              ? `条目 #${r.post_id}`
                              : "无条目编号"}{" "}
                        · {r.record_id.slice(0, 10)}
                      </option>
                    ))}
                  </select>
                </label>
                <div className="metadata-paging">
                  {state.focused && (
                    <button onClick={state.browseHistory}>
                      浏览此帖其他观察
                    </button>
                  )}
                  <span>本页 {data.records.length} 条记录</span>
                  {state.recordCursor && (
                    <button onClick={() => state.pageRecords()}>
                      首批记录
                    </button>
                  )}
                  {data.next_cursor && (
                    <button
                      onClick={() => state.pageRecords(data.next_cursor!)}
                    >
                      后续记录
                    </button>
                  )}
                </div>
                {observations.isPending && (
                  <p className="metadata-message" role="status">
                    正在读取历史观察…
                  </p>
                )}
                {observations.error && (
                  <Failure error={observations.error} refresh={state.refresh} />
                )}
                {!observations.error && observations.data && (
                  <>
                    {!observations.data.items.length && (
                      <p className="metadata-message">
                        这条来源记录没有可读取的观察。
                      </p>
                    )}
                    {observation && (
                      <>
                        <label className="metadata-label">
                          历史观察
                          <select
                            aria-label="历史观察"
                            value={observation.observation_id}
                            onChange={(e) =>
                              state.selectObservation(e.target.value)
                            }
                          >
                            {observations.data.items.map((o) => (
                              <option
                                key={o.observation_id}
                                value={o.observation_id}
                              >
                                {o.source_kind === "pinterest_pin_detail"
                                  ? "Pin 详情 · "
                                  : o.source_kind === "pinterest_media_manifest"
                                    ? "原图记录 · "
                                    : ""}
                                {o.observed_at?.slice(0, 10) ?? "时间未知"} ·{" "}
                                {o.relation === "asset_origin"
                                  ? "直接关联"
                                  : o.relation === "same_work"
                                    ? "同作品历史"
                                    : "同条目历史"}{" "}
                                · 行 {o.row_id}
                              </option>
                            ))}
                          </select>
                        </label>
                        <div className="metadata-paging">
                          <span>
                            本页 {observations.data.items.length} 次观察
                          </span>
                          {state.observationCursor && (
                            <button onClick={() => state.pageObservations()}>
                              首批观察
                            </button>
                          )}
                          {observations.data.next_cursor && (
                            <button
                              onClick={() =>
                                state.pageObservations(
                                  observations.data!.next_cursor!,
                                )
                              }
                            >
                              后续观察
                            </button>
                          )}
                        </div>
                        {["same_post", "same_pin"].includes(
                          observation.relation,
                        ) && (
                          <p className="metadata-message">
                            此观察属于同一来源条目，图片内容可能已变化。
                          </p>
                        )}
                        <dl className="metadata-fields">
                          <div className="metadata-field">
                            <dt>观察时间</dt>
                            <dd>
                              {observation.observed_at ?? "未记录"}
                              <small className="metadata-unknown">
                                {quality[
                                  observation.time_quality ?? "unknown"
                                ] ?? observation.time_quality}
                              </small>
                            </dd>
                          </div>
                          {observation.fields
                            .filter(
                              (f) =>
                                ![
                                  "rating",
                                  "tags",
                                  "source_width",
                                  "source_height",
                                ].includes(f.name) &&
                                !f.name.startsWith("tags.") &&
                                !f.name.includes("."),
                            )
                            .map((f) => (
                              <Field
                                key={f.name}
                                field={f}
                                onTag={onFilterTag}
                              />
                            ))}
                        </dl>
                        <details className="metadata-details">
                          <summary>分类标签与站点字段</summary>
                          <dl className="metadata-fields">
                            {observation.fields
                              .filter(
                                (f) =>
                                  f.name.startsWith("tags.") ||
                                  f.name.includes("."),
                              )
                              .map((f) => (
                                <Field
                                  key={f.name}
                                  field={f}
                                  onTag={onFilterTag}
                                />
                              ))}
                          </dl>
                        </details>
                        <details className="metadata-details">
                          <summary>记录依据</summary>
                          <dl className="metadata-fields">
                            <div className="metadata-field">
                              <dt>来源记录 ID</dt>
                              <dd>{record?.record_id}</dd>
                            </div>
                            <div className="metadata-field">
                              <dt>直接观察 ID</dt>
                              <dd>
                                {record?.origin_observation_id ?? "未记录"}
                              </dd>
                            </div>
                            <div className="metadata-field">
                              <dt>当前观察 ID</dt>
                              <dd>{observation.observation_id}</dd>
                            </div>
                            <div className="metadata-field">
                              <dt>数据来源</dt>
                              <dd>
                                {observation.source_kind ?? "未记录"}
                                <small>{observation.source_key}</small>
                              </dd>
                            </div>
                            <div className="metadata-field">
                              <dt>入库时间</dt>
                              <dd>{observation.ingested_at ?? "未记录"}</dd>
                            </div>
                            <div className="metadata-field">
                              <dt>存储配置</dt>
                              <dd>{record?.storage_profile ?? "未记录"}</dd>
                            </div>
                          </dl>
                          <p>
                            字段悬停可查看来源列。来源尺寸来自这次观察；存储尺寸需另行检查图片。
                          </p>
                        </details>
                        <div className="metadata-raw">
                          <p className="metadata-basis">
                            原始元数据：帖子 #{observation.post_id ?? "未知"} ·{" "}
                            {observation.observed_at?.slice(0, 10) ??
                              "观察时间未知"}
                          </p>
                          <Button
                            onClick={state.requestRaw}
                            disabled={
                              state.rawRequested ||
                              (!!ranking?.record_id && snapshot.isPending)
                            }
                          >
                            <FileJson2 size={13} />
                            读取原始元数据
                          </Button>
                          {state.rawRequested && raw.isPending && (
                            <p role="status">正在读取原始元数据…</p>
                          )}
                          {state.rawRequested && raw.error && (
                            <Failure
                              error={raw.error}
                              refresh={state.refresh}
                            />
                          )}
                          {state.rawRequested && !raw.error && raw.data && (
                            <>
                              {raw.data.status === "missing" && (
                                <p className="metadata-unknown">
                                  这次观察未保存原始元数据。
                                </p>
                              )}
                              {raw.data.status === "too_large" && (
                                <p className="metadata-message">
                                  原始记录为 {raw.data.bytes} 字节，超过 128 KiB
                                  查看上限。
                                </p>
                              )}
                              {raw.data.status === "available" && (
                                <>
                                  <small>
                                    {raw.data.format} · {raw.data.bytes} 字节
                                  </small>
                                  <pre tabIndex={0} aria-label="原始元数据">
                                    {rawText(raw.data.json ?? "")}
                                  </pre>
                                  {raw.data.schema && (
                                    <details>
                                      <summary>原始字段架构</summary>
                                      <small>
                                        {raw.data.schema.format} ·{" "}
                                        {raw.data.schema.bytes} 字节 ·{" "}
                                        {raw.data.schema.encoding}
                                      </small>
                                      <pre
                                        tabIndex={0}
                                        aria-label="原始字段架构"
                                      >
                                        {raw.data.schema.data ??
                                          "架构超过 64 KiB 查看上限，原始文件仍保留在数据湖中。"}
                                      </pre>
                                    </details>
                                  )}
                                  {raw.data.schema_id && (
                                    <small>
                                      Schema ID: {raw.data.schema_id}
                                    </small>
                                  )}
                                </>
                              )}
                            </>
                          )}
                        </div>
                      </>
                    )}
                  </>
                )}
              </>
            )}
          </details>
          <details className="metadata-details metadata-version">
            <summary>读取版本 · {data.version.analysis_sequence}</summary>
            <p>{data.version.generation}</p>
            <p>
              存储水位 {data.version.catalog_sequence} · 元数据水位{" "}
              {data.version.analysis_sequence}
            </p>
            <p>
              本次请求分别使用只读事务并核对水位；刷新会重新读取，不提供历史快照。
            </p>
          </details>
        </>
      )}
    </section>
  );
}
