import { useState } from "react";
import { Plus } from "lucide-react";
import { DraftStatus, useDraft } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import EvaluationPanel from "./EvaluationPanel.js";
import { RankingWorkspace } from "./RankingWorkspace.js";
import { ComparisonWorkspace } from "./ComparisonWorkspace.js";
import { ExperimentWorkspace } from "./ExperimentWorkspace.js";
import {
  rankingBrowserInitial,
  decodeRankingBrowser,
} from "./rankingBrowserState.js";
import "./aesthetic.css";
const initial = {
  view: "ranking" as
    "ranking" | "evaluation" | "protected" | "comparison" | "experiments",
};
const views: [typeof initial.view, string][] = [
  ["ranking", "排名浏览"],
  ["evaluation", "评审阶段"],
  ["protected", "保护复核"],
  ["comparison", "实验对照"],
  ["experiments", "实验配置"],
];
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  return views.some(([view]) => view === v.view) ? v : null;
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
  const [evaluationTarget, setEvaluationTarget] = useState<string>();
  const [fitTarget, setFitTarget] = useState<string>();
  const [analysisTarget, setAnalysisTarget] = useState<string>();
  const [comparisonTarget, setComparisonTarget] = useState<{
    left?: string;
    right?: string;
  }>();
  const rankingSession = useDraft(
    context.client,
    context.projectId,
    "core.aesthetic",
    rankingBrowserInitial,
    decodeRankingBrowser,
    "ranking-browser",
  );
  const protectedSession = useDraft(
    context.client,
    context.projectId,
    "core.aesthetic",
    rankingBrowserInitial,
    decodeRankingBrowser,
    "protected-browser",
  );
  function view(value: typeof initial.view) {
    const source =
      session.value.view === "ranking"
        ? rankingSession
        : session.value.view === "protected"
          ? protectedSession
          : null;
    const target =
      value === "ranking"
        ? rankingSession
        : value === "protected"
          ? protectedSession
          : null;
    if (
      source &&
      target &&
      source !== target &&
      source.editable &&
      target.editable &&
      source.value.snapshotId &&
      (source.value.snapshotId !== target.value.snapshotId ||
        source.value.ratings.join(",") !== target.value.ratings.join(","))
    ) {
      target.controller.set((old) => ({
        ...old,
        snapshotId: source.value.snapshotId,
        ratings: source.value.ratings,
        after: "",
        past: [],
        page: 1,
        ordinal: null,
        image: false,
        scrollTop: 0,
      }));
    }
    if (value !== "evaluation") setEvaluationTarget(undefined);
    if (value !== "comparison") setComparisonTarget(undefined);
    if (value !== "ranking") {
      setFitTarget(undefined);
      setAnalysisTarget(undefined);
    }
    if (session.editable) session.controller.set({ view: value });
  }
  const toolbarStart = (
    <>
      <div
        className="aesthetic-view-tabs"
        role="group"
        aria-label="美学工作视图"
      >
        {views.map(([value, label]) => (
          <button
            key={value}
            type="button"
            data-aesthetic-view={value}
            aria-pressed={session.value.view === value}
            disabled={!session.editable}
            onClick={() => {
              if (session.value.view !== value) view(value);
            }}
          >
            {label}
          </button>
        ))}
      </div>
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
        (session.value.view === "experiments" ? (
          <ExperimentWorkspace
            context={context}
            toolbarStart={toolbarStart}
            onRanking={(id) => {
              rankingSession.controller.set({
                ...rankingBrowserInitial,
                snapshotId: id,
              });
              view("ranking");
            }}
            onCompare={(left, right) => {
              setComparisonTarget({ left, right });
              view("comparison");
            }}
          />
        ) : session.value.view === "comparison" ? (
          <ComparisonWorkspace
            context={context}
            toolbarStart={toolbarStart}
            openComparison={comparisonTarget}
          />
        ) : session.value.view === "evaluation" ? (
          <EvaluationPanel
            {...context}
            toolbarStart={toolbarStart}
            openStageId={evaluationTarget}
            creating={creating}
            onCloseCreation={() => setCreating(false)}
            onRankings={async (stageId) => {
              const snapshot =
                await context.client.aesthetic.analysis.latestForStage(
                  context.projectId,
                  stageId,
                );
              const rating =
                snapshot?.result?.kind === "fit"
                  ? snapshot.result.groups.find((g) => g.compared > 0)?.rating
                  : undefined;
              rankingSession.controller.set({
                ...rankingBrowserInitial,
                snapshotId: snapshot?.id ?? "",
                ratings: [rating ?? "g"],
              });
              await rankingSession.controller.flush();
              setFitTarget(snapshot ? undefined : stageId);
              setAnalysisTarget(undefined);
              view("ranking");
            }}
            onAnalysisJob={(job) => {
              rankingSession.controller.set({ ...rankingBrowserInitial });
              setAnalysisTarget(job.id);
              setFitTarget(undefined);
              view("ranking");
            }}
          />
        ) : (
          <RankingWorkspace
            key={session.value.view}
            context={context}
            toolbarStart={toolbarStart}
            protectedOnly={session.value.view === "protected"}
            initialFitStageId={fitTarget}
            initialJobId={analysisTarget}
            onEvaluation={(stageId) => {
              setEvaluationTarget(stageId);
              view("evaluation");
            }}
            onCompare={(side, id) => {
              setComparisonTarget({ [side]: id });
              view("comparison");
            }}
          />
        ))}
    </section>
  );
}
