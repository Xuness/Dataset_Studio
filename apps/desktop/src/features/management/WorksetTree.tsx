import { FolderOpen } from "lucide-react";
import { Button, ErrorDetails, MoreMenu } from "@studio/ui";
import type {
  StudioClient,
  ObjectTarget,
  ObjectListOptions,
} from "@studio/client";
import type { ManagedObject } from "@studio/contracts";
import type { ManagementMode } from "./ManagementPanel.js";
import { useObjectList } from "./useObjectList.js";
export function WorksetTree({
  client,
  projectId,
  activeId,
  onBrowse,
  onManage,
}: {
  client: StudioClient;
  projectId: string;
  activeId: string | null;
  onBrowse: (item: ManagedObject) => void;
  onManage: (target: ObjectTarget, mode?: ManagementMode) => void;
}) {
  const list = useObjectList(client, projectId, "workset");
  return (
    <div className="workset-tree" aria-label="工作集列表">
      <div className="object-list-tools">
        <input
          type="search"
          aria-label="搜索工作集"
          placeholder="搜索工作集名称或备注"
          value={list.search}
          maxLength={120}
          onChange={(event) => list.searchFor(event.target.value)}
        />
        <select
          aria-label="工作集排序"
          value={list.order}
          onChange={(event) =>
            list.sortBy(event.target.value as ObjectListOptions["order"])
          }
        >
          <option value="created_desc">最近创建</option>
          <option value="created_asc">最早创建</option>
          <option value="name_asc">名称升序</option>
          <option value="name_desc">名称降序</option>
          <option value="count_desc">成员最多</option>
        </select>
      </div>
      {list.query.error && <ErrorDetails error={list.query.error} compact />}
      {list.query.isPending && (
        <p className="tree-hint" role="status">
          正在读取工作集…
        </p>
      )}
      {list.query.data?.items.map((item) => (
        <div className="managed-list-row" key={item.id}>
          <button
            type="button"
            className={"tree-row " + (item.id === activeId ? "active" : "")}
            title={item.notes || item.name}
            onClick={() => onBrowse(item)}
          >
            <FolderOpen size={14} />
            <span>{item.name}</span>
            <small>{item.count?.toLocaleString("zh-CN")}</small>
          </button>
          <MoreMenu
            label={item.name}
            items={[
              { label: "管理与来源详情", action: () => onManage(item) },
              {
                label: "重命名与备注…",
                action: () => onManage(item, "rename"),
              },
              {
                label: "删除工作集…",
                danger: true,
                action: () => onManage(item, "remove"),
              },
            ]}
          />
        </div>
      ))}
      {list.query.data && !list.query.data.items.length && (
        <p className="tree-hint">
          {list.search
            ? "没有匹配的工作集。"
            : "选择资料后，可将它们保存为工作集。"}
        </p>
      )}
      {(list.cursor || list.query.data?.next_cursor) && (
        <div className="managed-list-pagination">
          <Button
            disabled={!list.cursor || list.query.isFetching}
            onClick={list.previous}
          >
            上一页
          </Button>
          <Button
            disabled={!list.query.data?.next_cursor || list.query.isFetching}
            onClick={list.next}
          >
            下一页
          </Button>
        </div>
      )}
    </div>
  );
}
