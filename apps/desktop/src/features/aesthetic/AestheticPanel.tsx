import { useState } from "react";
import { Plus } from "lucide-react";
import { DraftStatus, useDraft } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import EvaluationPanel from "./EvaluationPanel.js";
import { RankingWorkspace } from "./RankingWorkspace.js";
import "./aesthetic.css";
const initial = { view: "ranking" as "ranking" | "evaluation" | "protected" };
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  return ["ranking", "evaluation", "protected"].includes(v.view) ? v : null;
}
export default function AestheticPanel(context: ModuleContext) {
  const session = useDraft(
    context.client,
    context.projectId,
    "core.aesthetic",
    initial,
    decode,
    "workbench-session",
  );
  const [creating, setCreating] = useState(false);
  function view(value: typeof initial.view) {
    if (session.editable) session.controller.set({ view: value });
  }
  const toolbarStart = (
    <>
      <label className="aesthetic-mode-control">
        <select
          aria-label="美学工作视图"
          value={session.value.view}
          disabled={!session.editable}
          onChange={(event) => view(event.target.value as typeof initial.view)}
        >
          <option value="ranking">排名浏览</option>
          <option value="evaluation">评审阶段</option>
          <option value="protected">保护复核</option>
        </select>
      </label>
      <span className="wb-tool-separator" />
      <button
        type="button"
        className="wb-tool-button"
        disabled={!session.editable}
        onClick={() => {
          view("evaluation");
          setCreating(true);
        }}
      >
        <Plus size={19} className="wb-create-icon" />
        新建评审
      </button>
      <span className="wb-tool-separator" />
    </>
  );
  return (
    <section className="aesthetic-workspace" aria-label="美学排序">
      <h2 className="wb-sr-only">美学排序</h2>
      <DraftStatus controller={session.controller} quiet />
      {session.editable &&
        (session.value.view === "evaluation" ? (
          <EvaluationPanel
            {...context}
            toolbarStart={toolbarStart}
            creating={creating}
            onCloseCreation={() => setCreating(false)}
            onRankings={() => view("ranking")}
          />
        ) : (
          <RankingWorkspace
            key={session.value.view}
            context={context}
            toolbarStart={toolbarStart}
            protectedOnly={session.value.view === "protected"}
            onEvaluation={() => view("evaluation")}
          />
        ))}
    </section>
  );
}
