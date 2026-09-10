import { lazy, Suspense, useEffect, useRef } from "react";
import { useDraft, ResizeGrip } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import BasicToolPanel from "./BasicToolPanel.js";
import "./tools.css";
const RankingPanel = lazy(() => import("../ranking/RankingPanel.js"));
const initial = { tool: "ranking" as "ranking" | "basic" };
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object" || !("tool" in value)) return null;
  return value.tool === "ranking" || value.tool === "basic"
    ? { tool: value.tool }
    : null;
}
export default function ToolPanel(context: ModuleContext) {
  const nav = useDraft(
    context.client,
    context.projectId,
    "core.tools",
    initial,
    decode,
    "navigation",
  );
  const applied = useRef<number | null>(null);
  useEffect(() => {
    const invocation = context.invocation;
    if (!nav.editable || !invocation || applied.current === invocation.sequence)
      return;
    const tool =
      invocation.args.operatorId === "danbooru.metarecall"
        ? "ranking"
        : "basic";
    if (invocation.args.operatorId) nav.controller.set({ tool });
    applied.current = invocation.sequence;
  }, [context.invocation, nav.controller, nav.editable]);
  return (
    <section className="tools-workspace" aria-label="计算工具">
      <nav className="tools-navigation" aria-label="计算模块">
        <button
          className={nav.value.tool === "ranking" ? "active" : ""}
          onClick={() => nav.controller.set({ tool: "ranking" })}
          disabled={!nav.editable}
        >
          Danbooru 元数据排名
        </button>
        <button
          className={nav.value.tool === "basic" ? "active" : ""}
          onClick={() => nav.controller.set({ tool: "basic" })}
          disabled={!nav.editable}
        >
          基础工具
        </button>
      </nav>
      {nav.value.tool === "basic" ? (
        <div className="tools-basic-content">
          <BasicToolPanel {...context} />
          {context.inspector?.visible && context.management && (
            <aside
              className="management-dock"
              style={{
                width: context.inspector.width,
                flexBasis: context.inspector.width,
              }}
            >
              {context.management.header}
              {context.management.tab === "management" ? (
                context.management.content
              ) : (
                <div className="management-empty">
                  选择计算工具与输入范围后可配置参数。
                  <br />
                  切换到“管理”可查看项目对象和来源关系。
                </div>
              )}
              <ResizeGrip
                label="管理面板宽度"
                orientation="vertical"
                reverse
                value={context.inspector.width}
                minimum={260}
                maximum={600}
                onChange={context.inspector.resize}
                onReset={() => context.inspector?.resize(320)}
              />
            </aside>
          )}
        </div>
      ) : (
        <Suspense
          fallback={<p className="ranking-loading">正在加载元数据排名…</p>}
        >
          <RankingPanel {...context} />
        </Suspense>
      )}
    </section>
  );
}
