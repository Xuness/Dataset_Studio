import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import { lakeKey, useLakeRefresh } from "./queries.js";

export function LakeRelocationSettings({ client }: { client: StudioClient }) {
  const refresh = useLakeRefresh(client);
  const lakes = useQuery({
    queryKey: [...lakeKey(client), "workspace-lakes"],
    queryFn: ({ signal }) =>
      client.sourceCollections.workspaceLakes({ signal, limit: 200 }),
    retry: false,
  });
  const moves = useQuery({
    queryKey: [...lakeKey(client), "relocations"],
    queryFn: ({ signal }) => client.lakeUpdates.relocations(signal),
    retry: false,
    refetchInterval: 5000,
  });
  const [selected, setSelected] = useState("");
  const [target, setTarget] = useState<{
    id: string;
    media_root: string;
    index_root: string;
  } | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [pending, setPending] = useState(false);
  const [notice, setNotice] = useState("");
  const lake = selected || lakes.data?.items[0]?.id || "";
  const location = lakes.data?.items.find((l) => l.id === lake);
  const move = moves.data?.items.find((m) => m.lake_id === lake);
  const switching =
    move?.phase === "verified" || move?.phase === "writer_committed";
  const paths =
    target && target.id === lake && !switching
      ? target
      : {
          id: lake,
          media_root: move?.media_root ?? location?.media ?? "",
          index_root: move?.index_root ?? location?.index_root ?? "",
        };
  async function run(action: () => Promise<string>) {
    setPending(true);
    setError(null);
    setNotice("");
    try {
      setNotice(await action());
    } catch (e) {
      setError(e);
    } finally {
      await refresh();
      setPending(false);
    }
  }
  async function reconnect() {
    const current =
      !move || move.phase === "draining"
        ? await client.lakeUpdates.prepareRelocation(lake)
        : move;
    if (current.phase === "draining") {
      return "更新操作尚未退出，请先在任务页暂停任务，然后重试";
    }
    await client.lakeUpdates.applyRelocation(current.id, {
      media_root: current.media_root ?? paths.media_root.trim(),
      index_root: current.index_root ?? paths.index_root.trim(),
    });
    setTarget(null);
    return "数据湖读写位置已同步，迁移完成";
  }
  return (
    <section className="lake-fields">
      <h3>迁移数据湖位置</h3>
      <p className="lake-hint">
        手动搬好目录后，填写新位置并重新关联。未搬动的目录保留原路径。程序只检查数据湖身份和索引版本，再同步所有项目与更新服务的位置。
      </p>
      <label>
        数据湖
        <select
          aria-label="迁移数据湖"
          value={lake}
          disabled={pending}
          onChange={(e) => {
            setSelected(e.target.value);
            setError(null);
            setNotice("");
          }}
        >
          {lakes.data?.items.map((l) => (
            <option key={l.id} value={l.id}>
              {l.site} · {l.id}
            </option>
          ))}
        </select>
      </label>
      {(lakes.error || moves.error) && (
        <ErrorDetails error={lakes.error || moves.error} />
      )}
      {move && (
        <p role="status">
          {(
            {
              draining: "等待更新操作退出，可暂停任务后重试或取消关联",
              prepared: "新写入已暂停，可以搬动目录或直接关联新位置",
              verified: "新位置已确认，继续完成关联",
              writer_committed: "正在同步项目位置，继续完成关联",
            } as Record<string, string>
          )[move.phase] ?? move.phase}
        </p>
      )}
      <label>
        迁移后的媒体目录
        <input
          aria-label="迁移后的媒体目录"
          value={paths.media_root}
          disabled={pending || switching}
          onChange={(e) => setTarget({ ...paths, media_root: e.target.value })}
        />
      </label>
      <label>
        迁移后的索引目录
        <input
          aria-label="迁移后的索引目录"
          value={paths.index_root}
          disabled={pending || switching}
          onChange={(e) => setTarget({ ...paths, index_root: e.target.value })}
        />
      </label>
      <Button
        disabled={
          pending ||
          !lake ||
          !paths.media_root.trim() ||
          !paths.index_root.trim()
        }
        onClick={() => void run(reconnect)}
      >
        {switching ? "继续完成关联" : "重新关联"}
      </Button>
      {!move && (
        <details>
          <summary>搬动文件前暂停写入</summary>
          <p className="lake-hint">
            如果文件还没搬，可以先暂停新写入。已有操作退出后，再手动搬动目录并重新关联。
          </p>
          <Button
            disabled={pending || !lake}
            onClick={() =>
              void run(async () => {
                const result = await client.lakeUpdates.prepareRelocation(lake);
                return result.phase === "draining"
                  ? "更新操作尚未退出，请先在任务页暂停任务，然后重试"
                  : "新写入已暂停，可以搬动目录";
              })
            }
          >
            暂停写入并准备搬迁
          </Button>
        </details>
      )}
      {move && ["draining", "prepared"].includes(move.phase) && (
        <Button
          disabled={pending}
          onClick={() =>
            void run(async () => {
              await client.lakeUpdates.cancelRelocation(move.id);
              setTarget(null);
              return "已取消关联，保留原登记路径；文件位置未改变";
            })
          }
        >
          取消关联
        </Button>
      )}
      {error != null && <ErrorDetails error={error} />}
      {notice && <p role="status">{notice}</p>}
    </section>
  );
}
