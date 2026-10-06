import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, DraftStatus, ErrorDetails } from "@studio/ui";
import { dateLabel, decodeDraft, initialDraft, sites } from "./model.js";
import type { Lake } from "./model.js";
import { lakeKey, useLakePreference } from "./queries.js";

type Filters = {
  all: string;
  any: string;
  none: string;
  start: string;
  end: string;
  missing: boolean;
};
const initialFilters: Filters = {
  all: "",
  any: "",
  none: "",
  start: "",
  end: "",
  missing: false,
};
const splitTags = (value: string) => [
  ...new Set(value.split(/[\s,，]+/).filter(Boolean)),
];
function definition(value: Filters): Schema["LakePostQuery"] {
  const integer = (text: string) => {
    if (!text.trim()) return undefined;
    const number = Number(text);
    if (
      !Number.isSafeInteger(number) ||
      number < 1 ||
      number === Number.MAX_SAFE_INTEGER
    )
      throw new Error("帖子 ID 必须是有效的正整数");
    return number;
  };
  const start = integer(value.start),
    end = integer(value.end);
  if (start != null && end != null && end < start)
    throw new Error("结束 ID 不能早于起始 ID");
  return {
    query: {
      all: splitTags(value.all),
      any: splitTags(value.any),
      none: splitTags(value.none),
    },
    start_id: start ?? null,
    end_id: end != null ? end + 1 : null,
    after: null,
    limit: null,
    version: null,
    post_ids: null,
    missing_media: value.missing,
  };
}

export function MetadataCatalog({
  client,
  lakes,
  initialLake,
  onUse,
}: {
  client: StudioClient;
  lakes: Lake[];
  initialLake: string;
  onUse: (libraryId: string) => void;
}) {
  const [selectedLake, setSelectedLake] = useState(initialLake);
  const libraryId = lakes.some((lake) => lake.id === selectedLake)
    ? selectedLake
    : (lakes[0]?.id ?? "");
  const [filters, setFilters] = useState(initialFilters);
  const [appliedFilters, setAppliedFilters] = useState(initialFilters);
  const [applied, setApplied] = useState<Schema["LakePostQuery"]>(() =>
    definition(initialFilters),
  );
  const [revision, setRevision] = useState(0);
  const [after, setAfter] = useState<number[]>([0]);
  const [selected, setSelected] = useState<number[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [pending, setPending] = useState(false);
  // Each search owns a snapshot cell; an old request cannot change a new search's version.
  const frozen = useMemo(
    () => ({ libraryId, revision, version: undefined as string | undefined }),
    [libraryId, revision],
  );
  const page = useQuery({
    queryKey: [
      ...lakeKey(client),
      "metadata-catalog",
      libraryId,
      applied,
      revision,
      after.at(-1),
    ],
    queryFn: async ({ signal }) => {
      const result = await client.lakeUpdates.catalog(
        libraryId,
        {
          ...applied,
          ...(frozen.version ? { version: frozen.version } : {}),
          after: after.at(-1) ?? 0,
          limit: 50,
        },
        signal,
      );
      frozen.version ??= result.version;
      return result;
    },
    enabled: !!libraryId,
    staleTime: Infinity,
    gcTime: 60_000,
    retry: false,
    refetchOnWindowFocus: false,
  });
  const form = useLakePreference(
    client,
    "studio.lake-updates.composer",
    initialDraft,
    decodeDraft,
  );
  function reset() {
    setAfter([0]);
    setSelected([]);
    setRevision((old) => old + 1);
  }
  function search() {
    try {
      setApplied(definition(filters));
      setAppliedFilters(filters);
      reset();
      setError(null);
    } catch (error) {
      setError(error);
    }
  }
  function toggle(id: number, checked: boolean) {
    setError(null);
    if (checked && selected.length >= 10000) {
      setError(new Error("一次最多选择 10000 个帖子"));
      return;
    }
    setSelected((old) =>
      checked
        ? [...new Set([...old, id])]
        : old.filter((value) => value !== id),
    );
  }
  async function prepare(onlySelected: boolean) {
    setPending(true);
    setError(null);
    try {
      if (!page.data || !libraryId) throw new Error("请先读取元数据目录");
      if (onlySelected && !selected.length) throw new Error("请先选择帖子");
      const old = form.value;
      if (old.submissions.some((submission) => !submission.jobId))
        throw new Error(
          "上一批更新仍有未确认的提交，请先在新建更新中完成确认。",
        );
      const query = applied.query;
      form.controller.set({
        ...old,
        lakes: [libraryId],
        kind: "tags",
        tagSource: "local",
        tagAll: query?.all?.join(" ") ?? "",
        tagAny: query?.any?.join(" ") ?? "",
        tagNone: query?.none?.join(" ") ?? "",
        tagMissingMedia: applied.missing_media ?? false,
        startId: applied.start_id?.toString() ?? "",
        endId: applied.end_id != null ? String(applied.end_id - 1) : "",
        perLake: {
          ...old.perLake,
          [libraryId]: {
            ...old.perLake[libraryId],
            metadataIds: onlySelected ? selected.join("\n") : "",
            catalogVersion: page.data.version,
          },
        },
        execution: "now",
        submissions: [],
      });
      await form.controller.flush();
      onUse(libraryId);
    } catch (error) {
      setError(error);
    } finally {
      setPending(false);
    }
  }
  if (!lakes.length)
    return (
      <p className="lake-empty">
        先创建或登记 Booru 数据湖；Pixiv 使用作者与作品采集入口。
      </p>
    );
  const items = page.data?.items ?? [];
  const dirty = JSON.stringify(filters) !== JSON.stringify(appliedFilters);
  return (
    <div className="lake-metadata-catalog">
      <DraftStatus controller={form.controller} quiet />
      <details open>
        <summary>帖子元数据 · 包含尚未下载图片的条目</summary>
        <div className="lake-metadata-filters">
          <label>
            元数据湖
            <select
              aria-label="元数据湖"
              value={libraryId}
              onChange={(event) => {
                setSelectedLake(event.target.value);
                reset();
              }}
            >
              {lakes.map((lake) => (
                <option key={lake.id} value={lake.id}>
                  {sites[lake.site]} · {lake.media}
                </option>
              ))}
            </select>
          </label>
          {(
            [
              ["all", "全部包含 Tag"],
              ["any", "任一包含 Tag"],
              ["none", "排除 Tag"],
            ] as const
          ).map(([field, label]) => (
            <label key={field}>
              {label}
              <input
                aria-label={`目录${label}`}
                value={filters[field]}
                onChange={(event) =>
                  setFilters({ ...filters, [field]: event.target.value })
                }
              />
            </label>
          ))}
          <label>
            起始 ID
            <input
              aria-label="目录起始 ID"
              inputMode="numeric"
              value={filters.start}
              onChange={(event) =>
                setFilters({ ...filters, start: event.target.value })
              }
            />
          </label>
          <label>
            结束 ID（含）
            <input
              aria-label="目录结束 ID"
              inputMode="numeric"
              value={filters.end}
              onChange={(event) =>
                setFilters({ ...filters, end: event.target.value })
              }
            />
          </label>
          <label className="lake-check">
            <input
              type="checkbox"
              checked={filters.missing}
              onChange={(event) =>
                setFilters({ ...filters, missing: event.target.checked })
              }
            />
            只看尚无图片的帖子
          </label>
          <Button onClick={search} disabled={pending}>
            查询元数据
          </Button>
        </div>
      </details>
      <div className="lake-pagination">
        <span>已选 {selected.length} 条</span>
        <Button
          disabled={
            page.isFetching ||
            !items.length ||
            selected.length +
              items.filter((item) => !selected.includes(item.post_id)).length >
              10000
          }
          onClick={() =>
            setSelected((old) => [
              ...new Set([...old, ...items.map((item) => item.post_id)]),
            ])
          }
        >
          选择本页
        </Button>
        <Button disabled={!selected.length} onClick={() => setSelected([])}>
          清空选择
        </Button>
        <span className="grow" />
        <Button
          disabled={
            dirty ||
            pending ||
            page.isFetching ||
            !form.editable ||
            !selected.length
          }
          onClick={() => void prepare(true)}
        >
          为已选帖子新建任务
        </Button>
        <Button
          disabled={
            dirty || pending || page.isFetching || !form.editable || !page.data
          }
          onClick={() => void prepare(false)}
        >
          为匹配元数据新建任务
        </Button>
      </div>
      {dirty && (
        <p className="lake-hint">
          筛选条件尚未应用，点击“查询元数据”更新列表。
        </p>
      )}
      {error != null && <ErrorDetails error={error} />}
      {page.error && <ErrorDetails error={page.error} />}
      <div className="lake-table-scroll">
        <table
          className="lake-table lake-metadata-table"
          aria-label="帖子元数据"
        >
          <thead>
            <tr>
              <th>选择</th>
              <th>帖子 ID</th>
              <th>图片</th>
              <th>分级 / 尺寸</th>
              <th>原获取时间</th>
              <th>标签</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => (
              <tr
                key={item.post_id}
                className={selected.includes(item.post_id) ? "selected" : ""}
              >
                <td>
                  <input
                    type="checkbox"
                    aria-label={`选择帖子 ${item.post_id}`}
                    checked={selected.includes(item.post_id)}
                    onChange={(event) =>
                      toggle(item.post_id, event.target.checked)
                    }
                  />
                </td>
                <td>{item.post_id}</td>
                <td>{item.has_media ? "已有图片" : "尚未下载"}</td>
                <td>
                  {item.rating ?? "未知"} · {item.width ?? "?"} ×{" "}
                  {item.height ?? "?"}
                </td>
                <td>{dateLabel(item.observed_at)}</td>
                <td className="lake-metadata-tags">
                  {item.tags ?? "标签未知"}
                  {item.tags_truncated ? " …" : ""}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!items.length && (
          <p className="lake-empty">
            {page.isFetching
              ? "正在读取元数据…"
              : page.data?.next_after != null
                ? "本批没有命中，可继续查询后面的帖子。"
                : "当前范围没有已入湖的帖子元数据。"}
          </p>
        )}
      </div>
      <footer className="lake-pagination">
        <span>
          第 {after.length} 页 · 本批检查 {page.data?.scanned ?? 0} 条 ·
          保留本次读取版本
        </span>
        <Button disabled={dirty || page.isFetching} onClick={reset}>
          刷新元数据版本
        </Button>
        <span className="grow" />
        <Button
          disabled={after.length === 1 || page.isFetching}
          onClick={() => setAfter((old) => old.slice(0, -1))}
        >
          上一页元数据
        </Button>
        <Button
          disabled={page.data?.next_after == null || page.isFetching}
          onClick={() => setAfter((old) => [...old, page.data!.next_after!])}
        >
          下一页元数据
        </Button>
      </footer>
    </div>
  );
}
