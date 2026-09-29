import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import { lakeKey, useLakeRefresh } from "./queries.js";

export function LakeRelocationSettings({ client }: { client: StudioClient }) {
  const refresh = useLakeRefresh(client);
  const lakes = useQuery({
    queryKey: [...lakeKey(client), "lakes"],
    queryFn: ({ signal }) => client.lakeUpdates.lakes(signal),
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
  const move = moves.data?.items.find((m) => m.lake_id === lake);
  const paths =
    target && target.id === move?.id
      ? target
      : {
          id: move?.id ?? "",
          media_root: move?.media_root ?? move?.old_media ?? "",
          index_root: move?.index_root ?? move?.old_index ?? "",
        };
  async function run(action: () => Promise<unknown>, message: string) {
    setPending(true);
    setError(null);
    setNotice("");
    try {
      await action();
      setNotice(message);
    } catch (e) {
      setError(e);
    } finally {
      await refresh();
      setPending(false);
    }
  }
  return (
    <section className="lake-fields">
      <h3>迁移数据湖位置</h3>
      <p className="lake-hint">
        先准备迁移，等待现有操作退出，再搬动或复制媒体与索引目录。验证通过后会同时切换所有项目和更新服务的位置。搬迁前请保留原目录或完整备份。
      </p>
      <label>
        数据湖
        <select
          aria-label="迁移数据湖"
          value={lake}
          disabled={pending}
          onChange={(e) => setSelected(e.target.value)}
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
      {!move ? (
        <Button
          disabled={pending || !lake}
          onClick={() =>
            void run(
              () => client.lakeUpdates.prepareRelocation(lake),
              "迁移准备已请求，请查看状态",
            )
          }
        >
          准备迁移并冻结写入
        </Button>
      ) : (
        <>
          <p role="status">
            迁移状态：
            {(
              {
                draining: "等待现有操作退出",
                prepared: "已准备，可以搬迁文件",
                verified: "新位置已验证，继续完成切换",
                writer_committed: "等待读侧登记完成",
              } as Record<string, string>
            )[move.phase] ?? move.phase}
          </p>
          {move.phase === "draining" ? (
            <>
              <p className="lake-hint">
                可在任务页暂停长任务，然后再次检查。尚未准备完成时不要搬动目录。
              </p>
              <Button
                disabled={pending}
                onClick={() =>
                  void run(
                    () => client.lakeUpdates.prepareRelocation(lake),
                    "已重新检查迁移准备",
                  )
                }
              >
                检查是否可以搬迁
              </Button>
            </>
          ) : (
            <>
              <label>
                迁移后的媒体目录
                <input
                  aria-label="迁移后的媒体目录"
                  value={paths.media_root}
                  disabled={pending || move.phase !== "prepared"}
                  onChange={(e) =>
                    setTarget({ ...paths, media_root: e.target.value })
                  }
                />
              </label>
              <label>
                迁移后的索引目录
                <input
                  aria-label="迁移后的索引目录"
                  value={paths.index_root}
                  disabled={pending || move.phase !== "prepared"}
                  onChange={(e) =>
                    setTarget({ ...paths, index_root: e.target.value })
                  }
                />
              </label>
              <Button
                disabled={pending || !paths.media_root || !paths.index_root}
                onClick={() =>
                  void run(
                    () =>
                      client.lakeUpdates.applyRelocation(move.id, {
                        media_root: paths.media_root,
                        index_root: paths.index_root,
                      }),
                    "数据湖读写位置已同步，迁移完成",
                  )
                }
              >
                验证并完成迁移
              </Button>
            </>
          )}
          {["draining", "prepared"].includes(move.phase) && (
            <Button
              disabled={pending}
              onClick={() =>
                void run(
                  () => client.lakeUpdates.cancelRelocation(move.id),
                  "迁移已取消，恢复原位置",
                )
              }
            >
              取消迁移并保留原位置
            </Button>
          )}
        </>
      )}
      {error != null && <ErrorDetails error={error} />}
      {notice && <p role="status">{notice}</p>}
    </section>
  );
}
