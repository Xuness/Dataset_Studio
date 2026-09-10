import { Database } from "lucide-react";
import { Button, ErrorDetails } from "@studio/ui";
import type { StudioClient, ObjectTarget } from "@studio/client";
import { useObjectList } from "./useObjectList.js";
export function DetachedSources({
  client,
  projectId,
  onManage,
}: {
  client: StudioClient;
  projectId: string;
  onManage: (target: ObjectTarget) => void;
}) {
  const list = useObjectList(client, projectId, "source", {
    state: "detached",
  });
  if (!list.query.data?.items.length && !list.query.error) return null;
  return (
    <details className="detached-sources">
      <summary>已取消关联的数据湖</summary>
      {list.query.error && <ErrorDetails error={list.query.error} compact />}
      {list.query.data?.items.map((item) => (
        <button
          key={item.id}
          className="tree-row"
          onClick={() => onManage(item)}
        >
          <Database size={13} />
          <span>{item.name}</span>
          <small>未关联</small>
        </button>
      ))}
      {list.query.data?.next_cursor && (
        <Button onClick={list.next}>更多数据湖</Button>
      )}
      {list.cursor && <Button onClick={list.previous}>上一页</Button>}
    </details>
  );
}
