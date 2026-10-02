import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import { Button, ErrorDetails } from "@studio/ui";

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}
export function PixivOrigin({
  client,
  projectId,
  sourceId,
  version,
  workId,
  ordinal,
  kind,
  representation,
  onFilter,
}: {
  client: StudioClient;
  projectId: string;
  sourceId: string;
  version: string;
  workId: string;
  ordinal: number | null | undefined;
  kind: string | null | undefined;
  representation: string | null | undefined;
  onFilter?:
    ((field: "work.id" | "author.id", value: string) => void) | undefined;
}) {
  const work = useQuery({
    queryKey: ["project", projectId, "pixiv-work", sourceId, workId, version],
    queryFn: ({ signal }) =>
      client.sourceAccess.work(projectId, sourceId, workId, {
        version,
        signal,
      }),
    staleTime: Infinity,
  });
  const data = record(work.data?.observation),
    authorId = typeof data.author_id === "string" ? data.author_id : "";
  const author = useQuery({
    queryKey: [
      "project",
      projectId,
      "pixiv-author",
      sourceId,
      authorId,
      version,
    ],
    queryFn: ({ signal }) =>
      client.sourceAccess.author(projectId, sourceId, authorId, {
        version,
        signal,
      }),
    enabled: !!authorId,
    retry: false,
    staleTime: Infinity,
  });
  const name = record(author.data?.observation).display_name;
  return (
    <section className="metadata-primary" aria-label="Pixiv 作品资料">
      <h4>{typeof data.title === "string" ? data.title : `作品 ${workId}`}</h4>
      <dl className="metadata-fields">
        <div className="metadata-field">
          <dt>作品</dt>
          <dd>{workId}</dd>
        </div>
        <div className="metadata-field">
          <dt>{kind === "ugoira" ? "动画" : "页序"}</dt>
          <dd>
            {kind === "ugoira"
              ? "Ugoira · 首帧封面"
              : `${(ordinal ?? 0) + 1} / ${typeof data.page_count === "number" ? data.page_count : "未知"}`}
          </dd>
        </div>
        <div className="metadata-field">
          <dt>作者</dt>
          <dd>
            {typeof name === "string"
              ? `${name} · ${authorId}`
              : authorId || "尚未读取"}
          </dd>
        </div>
        <div className="metadata-field">
          <dt>文件</dt>
          <dd>
            {representation === "original"
              ? "原图"
              : representation === "poster"
                ? "动画封面"
                : "派生图片"}
          </dd>
        </div>
      </dl>
      <div className="metadata-paging">
        <Button
          disabled={!onFilter}
          onClick={() => onFilter?.("work.id", workId)}
        >
          筛选同作品
        </Button>
        <Button
          disabled={!onFilter || !authorId}
          onClick={() => onFilter?.("author.id", authorId)}
        >
          筛选同作者
        </Button>
        <a
          href={`https://www.pixiv.net/artworks/${workId}`}
          target="_blank"
          rel="noreferrer"
        >
          打开 Pixiv 作品
        </a>
      </div>
      {work.error && <ErrorDetails error={work.error} />}
    </section>
  );
}
