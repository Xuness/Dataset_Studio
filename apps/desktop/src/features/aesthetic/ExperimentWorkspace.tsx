import { useRef, useState } from "react";
import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Schema } from "@studio/contracts";
import { StudioError } from "@studio/client";
import {
  DraftStatus,
  ErrorDetails,
  useDraft,
  useWorkbenchLayout,
  Workbench,
  WorkbenchPreferences,
} from "@studio/ui";
import type { ModuleContext, WorkbenchLayout } from "@studio/ui";
import { analysisActive, analysisState } from "./analysisPresentation.js";

type Variant = Schema["AestheticExperimentVariant"];
function variant(label: string, stageId = ""): Variant {
  return {
    label,
    fit: {
      stage_id: stageId,
      estimator: {
        kind: "davidson_v1",
        iterations: 128,
        regularization: 0.1,
        tie_strength: 1,
      },
      stability_seed: 17,
    },
  };
}
const initial = {
  name: "离线排序实验",
  description: "",
  variants: [
    variant("Davidson"),
    {
      ...variant("Borda"),
      fit: {
        ...variant("Borda").fit,
        estimator: { ...variant("Borda").fit.estimator, kind: "borda_v1" },
      },
    },
  ],
  key: "",
  editing: true,
  selectedId: "",
  selectedVariant: 0,
  left: "",
  right: "",
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  return [v.name, v.description, v.key, v.selectedId, v.left, v.right].every(
    (s) => typeof s === "string",
  ) &&
    typeof v.editing === "boolean" &&
    Number.isInteger(v.selectedVariant) &&
    v.selectedVariant >= 0 &&
    v.selectedVariant < 12 &&
    Array.isArray(v.variants) &&
    v.variants.length >= 1 &&
    v.variants.length <= 12 &&
    v.variants.every(
      (item) =>
        typeof item.label === "string" &&
        typeof item.fit?.stage_id === "string" &&
        ["borda_v1", "davidson_v1"].includes(item.fit.estimator?.kind) &&
        [
          item.fit.estimator.iterations,
          item.fit.estimator.regularization,
          item.fit.estimator.tie_strength,
        ].every(Number.isFinite) &&
        (item.fit.stability_seed === null ||
          Number.isInteger(item.fit.stability_seed)),
    )
    ? v
    : null;
}
const initialLayout: WorkbenchLayout = {
  panels: { experiments: "left", definition: "right" },
  active: {},
  leftWidth: 220,
  rightWidth: 380,
  bottomHeight: 260,
};
export function ExperimentWorkspace({
  context,
  toolbarStart,
  onRanking,
  onCompare,
}: {
  context: ModuleContext;
  toolbarStart: ReactNode;
  onRanking: (id: string) => void;
  onCompare: (left: string, right: string) => void;
}) {
  const { client, projectId } = context;
  const api = client.aesthetic.analysis;
  const draft = useDraft(
    client,
    projectId,
    "core.aesthetic",
    initial,
    decode,
    "experiment-editor",
  );
  const layout = useWorkbenchLayout(
    client,
    "aesthetic-experiments",
    initialLayout,
  );
  const saved = draft.value;
  const [after, setAfter] = useState("");
  const [stageAfter, setStageAfter] = useState("");
  const [jobAfter, setJobAfter] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const lock = useRef(false);
  const experiments = useQuery({
    queryKey: ["project", projectId, "aesthetic", "experiments", after],
    queryFn: ({ signal }) =>
      api.experiments(projectId, after || undefined, signal),
  });
  const selected = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "experiment",
      saved.selectedId,
    ],
    queryFn: ({ signal }) =>
      api.experiment(projectId, saved.selectedId, signal),
    enabled: !!saved.selectedId && !saved.editing,
  });
  const jobs = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "experiment-jobs",
      saved.selectedId,
      jobAfter,
    ],
    queryFn: ({ signal }) =>
      api.jobs(
        projectId,
        { experiment_id: saved.selectedId, after: jobAfter, limit: 32 },
        signal,
      ),
    enabled: !!saved.selectedId && !saved.editing,
    refetchInterval: (q) =>
      q.state.data?.items.some((j) => analysisActive(j.state)) ? 1000 : false,
  });
  const stages = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "experiment-stages",
      stageAfter,
    ],
    queryFn: ({ signal }) =>
      client.aesthetic.stages(projectId, stageAfter || undefined, signal),
    enabled: saved.editing,
  });
  const index = Math.min(saved.selectedVariant, saved.variants.length - 1);
  const current = saved.variants[index]!;
  const stage = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "stage",
      current.fit.stage_id,
    ],
    queryFn: ({ signal }) =>
      client.aesthetic.stage(projectId, current.fit.stage_id, signal),
    enabled: saved.editing && !!current.fit.stage_id,
  });
  const choices = [...(stages.data?.items ?? [])];
  if (stage.data && !choices.some((s) => s.id === stage.data.id))
    choices.unshift(stage.data);
  const editable = draft.editable && !busy && !saved.key;
  function edit(patch: Partial<typeof initial>) {
    if (editable) draft.controller.set((old) => ({ ...old, ...patch }));
  }
  function editVariant(patch: Partial<Variant>) {
    edit({
      variants: saved.variants.map((v, i) =>
        i === index ? { ...v, ...patch } : v,
      ),
    });
  }
  function estimator(patch: Partial<Variant["fit"]["estimator"]>) {
    editVariant({
      fit: {
        ...current.fit,
        estimator: { ...current.fit.estimator, ...patch },
      },
    });
  }
  async function run(action: () => Promise<void>) {
    if (lock.current || !draft.editable) return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  async function create() {
    const key = saved.key || crypto.randomUUID();
    draft.controller.set((old) => ({ ...old, key }));
    await client.edits.flush(projectId);
    let result: Schema["AestheticExperiment"];
    try {
      result = await api.createExperiment(projectId, {
        idempotency_key: key,
        name: saved.name.trim(),
        description: saved.description,
        variants: saved.variants,
      });
    } catch (e) {
      if (
        e instanceof StudioError &&
        ["INVALID_INPUT", "NOT_FOUND", "EVALUATION_NO_EVIDENCE"].includes(
          e.code,
        )
      )
        draft.controller.set((old) => ({ ...old, key: "" }));
      throw e;
    }
    draft.controller.set((old) => ({
      ...old,
      key: "",
      selectedId: result.id,
      editing: false,
      left: "",
      right: "",
    }));
    await draft.controller.flush();
    setJobAfter("");
    await experiments.refetch();
  }
  const completed =
    jobs.data?.items.filter(
      (j) => j.state === "completed" && j.request.spec.kind === "fit",
    ) ?? [];
  const config = saved.editing ? (
    <form
      className="wb-field-list experiment-editor"
      onSubmit={(e) => {
        e.preventDefault();
        void run(create);
      }}
    >
      <label>
        实验名称
        <input
          value={saved.name}
          required
          maxLength={120}
          disabled={!editable}
          onChange={(e) => edit({ name: e.target.value })}
        />
      </label>
      <label>
        实验说明
        <textarea
          value={saved.description}
          maxLength={2000}
          rows={3}
          disabled={!editable}
          onChange={(e) => edit({ description: e.target.value })}
        />
      </label>
      <h4>
        变体 {index + 1} / {saved.variants.length}
      </h4>
      <label>
        变体名称
        <input
          aria-label="变体名称"
          value={current.label}
          maxLength={120}
          required
          disabled={!editable}
          onChange={(e) => editVariant({ label: e.target.value })}
        />
      </label>
      <label>
        来源评审阶段
        <select
          aria-label="实验来源阶段"
          value={current.fit.stage_id}
          required
          disabled={!editable}
          onChange={(e) =>
            editVariant({ fit: { ...current.fit, stage_id: e.target.value } })
          }
        >
          <option value="">选择已有有效证据的阶段</option>
          {choices.map((s) => (
            <option key={s.id} value={s.id} disabled={!s.accepted}>
              {s.name} · {s.accepted} 批有效
            </option>
          ))}
        </select>
      </label>
      <div className="aesthetic-actions">
        <button
          type="button"
          disabled={busy || !stageAfter}
          onClick={() => setStageAfter("")}
        >
          阶段首页
        </button>
        <button
          type="button"
          disabled={busy || !stages.data?.next_cursor}
          onClick={() => setStageAfter(stages.data?.next_cursor ?? "")}
        >
          更多阶段
        </button>
        <button
          type="button"
          disabled={!editable || !current.fit.stage_id}
          onClick={() =>
            edit({
              variants: saved.variants.map((v) => ({
                ...v,
                fit: { ...v.fit, stage_id: current.fit.stage_id },
              })),
            })
          }
        >
          所有变体使用此阶段
        </button>
      </div>
      <label>
        估计器
        <select
          aria-label="变体估计器"
          value={current.fit.estimator.kind}
          disabled={!editable}
          onChange={(e) => estimator({ kind: e.target.value })}
        >
          <option value="davidson_v1">Davidson</option>
          <option value="borda_v1">Borda 基线</option>
        </select>
      </label>
      <details className="wb-fold" open>
        <summary>计算参数</summary>
        <div className="wb-field-list">
          {(
            [
              ["iterations", "迭代上限", 1, 128, 1],
              ["regularization", "正则化", 0.001, 10, "any"],
              ["tie_strength", "并列强度", 0.01, 100, "any"],
            ] as const
          ).map(([key, label, min, max, step]) => (
            <label key={key}>
              {label}
              <input
                type="number"
                required
                min={min}
                max={max}
                step={step}
                disabled={!editable}
                value={current.fit.estimator[key]}
                onChange={(e) => estimator({ [key]: Number(e.target.value) })}
              />
            </label>
          ))}
          <label className="experiment-check">
            <input
              type="checkbox"
              checked={current.fit.stability_seed !== null}
              disabled={!editable}
              onChange={(e) =>
                editVariant({
                  fit: {
                    ...current.fit,
                    stability_seed: e.target.checked ? 17 : null,
                  },
                })
              }
            />
            分半稳定性重放
          </label>
          {current.fit.stability_seed !== null && (
            <label>
              分半种子
              <input
                type="number"
                min={0}
                max={4294967295}
                step={1}
                required
                value={current.fit.stability_seed}
                disabled={!editable}
                onChange={(e) =>
                  editVariant({
                    fit: {
                      ...current.fit,
                      stability_seed: Number(e.target.value),
                    },
                  })
                }
              />
            </label>
          )}
        </div>
      </details>
      <p className="aesthetic-help">
        保存时冻结各阶段已有证据。运行变体仅做离线重算，不调用模型。比较模型或
        Prompt 时，请分别选择对应的评审阶段。
      </p>
      {saved.key && <p>正在确认上一份实验定义；可重试同一请求。</p>}
      <button
        type="submit"
        disabled={
          busy ||
          !draft.editable ||
          !saved.name.trim() ||
          saved.variants.some((v) => !v.label.trim() || !v.fit.stage_id) ||
          new Set(saved.variants.map((v) => v.label)).size !==
            saved.variants.length
        }
      >
        {saved.key ? "恢复实验创建" : "保存实验定义"}
      </button>
    </form>
  ) : (
    <div className="wb-field-list">
      <h4>{selected.data?.request.name}</h4>
      <p>{selected.data?.request.description}</p>
      <p className="aesthetic-help">
        实验定义与证据水位已经冻结。重复运行会找回同一批离线任务。
      </p>
      <button
        disabled={busy || !selected.data}
        onClick={() =>
          void run(async () => {
            await api.runExperiment(projectId, saved.selectedId);
            await jobs.refetch();
          })
        }
      >
        运行全部变体
      </button>
      <button
        disabled={busy || !selected.data || !!saved.key}
        onClick={() => {
          const request = selected.data!.request;
          draft.controller.set((old) => ({
            ...old,
            name: request.name + " 副本",
            description: request.description,
            variants: request.variants,
            editing: true,
            selectedVariant: 0,
            key: "",
          }));
        }}
      >
        复制为新实验
      </button>
      <details className="wb-fold">
        <summary>冻结依据</summary>
        {selected.data?.inputs.map((input, i) => (
          <dl className="wb-property-list" key={i}>
            <dt>变体</dt>
            <dd>{selected.data?.request.variants[i]?.label}</dd>
            <dt>阶段</dt>
            <dd>{input.stage_id}</dd>
            <dt>接受批次</dt>
            <dd>{input.observations}</dd>
            <dt>证据水位</dt>
            <dd>{input.evidence_watermark}</dd>
          </dl>
        ))}
      </details>
      {completed.length >= 2 && (
        <>
          <label>
            对照 A
            <select
              aria-label="实验对照 A"
              value={saved.left}
              disabled={busy}
              onChange={(e) =>
                draft.controller.set((old) => ({
                  ...old,
                  left: e.target.value,
                }))
              }
            >
              <option value="">选择快照</option>
              {completed.map((j) => (
                <option key={j.id} value={j.id}>
                  {j.request.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            对照 B
            <select
              aria-label="实验对照 B"
              value={saved.right}
              disabled={busy}
              onChange={(e) =>
                draft.controller.set((old) => ({
                  ...old,
                  right: e.target.value,
                }))
              }
            >
              <option value="">选择快照</option>
              {completed.map((j) => (
                <option key={j.id} value={j.id}>
                  {j.request.name}
                </option>
              ))}
            </select>
          </label>
          <button
            disabled={
              busy || !saved.left || !saved.right || saved.left === saved.right
            }
            onClick={() => onCompare(saved.left, saved.right)}
          >
            在实验对照中打开
          </button>
        </>
      )}
    </div>
  );
  return (
    <Workbench
      title="实验配置工作台"
      layout={{
        ...layout.value,
        panels: { ...initialLayout.panels, ...layout.value.panels },
      }}
      onLayout={layout.update}
      disabled={!layout.editable}
      toolbar={
        <>
          {toolbarStart}
          <button
            disabled={!editable}
            onClick={() =>
              draft.controller.set({
                ...initial,
                variants: initial.variants.map((v) => structuredClone(v)),
              })
            }
          >
            新建离线实验
          </button>
          <span className="grow" />
          <WorkbenchPreferences state={layout} />
        </>
      }
      status={<DraftStatus controller={draft.controller} quiet />}
      panels={[
        {
          id: "experiments",
          title: "离线实验",
          content: (
            <div className="evaluation-stage-tree">
              {experiments.data?.items.map((item) => (
                <button
                  type="button"
                  key={item.id}
                  className={
                    "experiment-list-item " +
                    (saved.selectedId === item.id && !saved.editing
                      ? "active"
                      : "")
                  }
                  disabled={!draft.editable || busy || !!saved.key}
                  onClick={() => {
                    draft.controller.set((old) => ({
                      ...old,
                      selectedId: item.id,
                      editing: false,
                      left: "",
                      right: "",
                    }));
                    setJobAfter("");
                  }}
                >
                  {item.request.name} · {item.request.variants.length} 个变体
                </button>
              ))}
              {!experiments.data?.items.length && <p>尚未保存实验。</p>}
              <div className="aesthetic-actions">
                <button disabled={busy || !after} onClick={() => setAfter("")}>
                  实验首页
                </button>
                <button
                  disabled={busy || !experiments.data?.next_cursor}
                  onClick={() => setAfter(experiments.data?.next_cursor ?? "")}
                >
                  更多实验
                </button>
              </div>
            </div>
          ),
        },
        {
          id: "definition",
          title: saved.editing ? "变体设置" : "实验详情",
          content: config,
        },
      ]}
    >
      <main className="experiment-main">
        {[
          error,
          experiments.error,
          selected.error,
          jobs.error,
          stages.error,
          stage.error,
        ]
          .filter(Boolean)
          .map((e, i) => (
            <ErrorDetails key={i} error={e} />
          ))}
        <h3>{saved.editing ? saved.name : selected.data?.request.name}</h3>
        {saved.editing ? (
          <>
            <p>选中变体，在右侧选择来源和计算参数；一次实验最多 12 个变体。</p>
            <div className="experiment-variants">
              {saved.variants.map((v, i) => (
                <button
                  type="button"
                  key={i}
                  disabled={busy}
                  aria-pressed={i === index}
                  onClick={() =>
                    draft.controller.set((old) => ({
                      ...old,
                      selectedVariant: i,
                    }))
                  }
                >
                  <strong>{v.label || "未命名变体"}</strong>
                  <span>
                    {v.fit.estimator.kind} ·{" "}
                    {v.fit.stage_id ? "已选择阶段" : "待选择阶段"}
                  </span>
                </button>
              ))}
            </div>
            <div className="aesthetic-actions">
              <button
                disabled={!editable || saved.variants.length >= 12}
                onClick={() => {
                  let n = saved.variants.length + 1;
                  while (saved.variants.some((v) => v.label === "变体 " + n))
                    n++;
                  edit({
                    variants: [
                      ...saved.variants,
                      { ...structuredClone(current), label: "变体 " + n },
                    ],
                    selectedVariant: saved.variants.length,
                  });
                }}
              >
                添加变体
              </button>
              <button
                disabled={!editable || saved.variants.length <= 1}
                onClick={() =>
                  edit({
                    variants: saved.variants.filter((_, i) => i !== index),
                    selectedVariant: Math.max(0, index - 1),
                  })
                }
              >
                移除当前变体
              </button>
            </div>
          </>
        ) : (
          <>
            <table className="experiment-results">
              <thead>
                <tr>
                  <th>变体</th>
                  <th>状态</th>
                  <th>进度</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {jobs.data?.items.map((job) => (
                  <tr key={job.id}>
                    <td>{job.request.name}</td>
                    <td>
                      {analysisState(job.state)}
                      {job.error && <p role="alert">{job.error}</p>}
                    </td>
                    <td>
                      {job.progress.toLocaleString()} /{" "}
                      {job.total.toLocaleString()}
                    </td>
                    <td>
                      {job.state === "completed" ? (
                        <button onClick={() => onRanking(job.id)}>
                          查看排名
                        </button>
                      ) : (
                        <button
                          disabled={busy}
                          onClick={() =>
                            void run(async () => {
                              await api.control(
                                projectId,
                                job.id,
                                analysisActive(job.state) ? "cancel" : "resume",
                              );
                              await jobs.refetch();
                            })
                          }
                        >
                          {analysisActive(job.state) ? "取消计算" : "恢复计算"}
                        </button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!jobs.data?.items.length && (
              <p>定义已保存，点击右侧“运行全部变体”生成快照。</p>
            )}
            <div className="aesthetic-actions">
              <button
                disabled={busy || !jobAfter}
                onClick={() => setJobAfter("")}
              >
                结果首页
              </button>
              <button
                disabled={busy || !jobs.data?.next_cursor}
                onClick={() => setJobAfter(jobs.data?.next_cursor ?? "")}
              >
                更多结果
              </button>
            </div>
          </>
        )}
      </main>
    </Workbench>
  );
}
