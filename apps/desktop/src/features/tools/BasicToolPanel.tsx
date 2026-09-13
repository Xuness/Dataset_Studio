import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Play, Calculator, RotateCw } from "lucide-react";
import { Button, Field, useDraft, DraftStatus, ErrorDetails } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import type {
  Artifact,
  OperatorDescriptor,
  ScalarInput,
  ScopeRef,
  OperatorRun,
} from "@studio/contracts";
import { ScopePicker } from "../scopes/ScopePicker.js";
import { PresetControls } from "./PresetControls.js";
import "./tools.css";

type ToolDraft = {
  operatorId: string;
  operatorVersion: number;
  parametersVersion: number;
  parameters: Record<string, unknown>;
  scope: ScopeRef | null;
  scopeId: string;
  submission: { key: string; signature: string } | null;
  lastJob: string | null;
};
const initial: ToolDraft = {
  operatorId: "core.manifest",
  operatorVersion: 1,
  parametersVersion: 1,
  parameters: { fields: [] },
  scope: null,
  scopeId: "",
  submission: null,
  lastJob: null,
};
function decode(value: unknown): ToolDraft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    typeof v.operatorId !== "string" ||
    typeof v.operatorVersion !== "number" ||
    typeof v.parametersVersion !== "number" ||
    !v.parameters ||
    typeof v.parameters !== "object" ||
    Array.isArray(v.parameters) ||
    typeof v.scopeId !== "string"
  )
    return null;
  if (
    v.scope !== null &&
    (!v.scope ||
      typeof v.scope !== "object" ||
      !("project_id" in v.scope) ||
      !("target" in v.scope))
  )
    return null;
  if (
    v.submission !== null &&
    (!v.submission ||
      typeof v.submission !== "object" ||
      !("key" in v.submission) ||
      !("signature" in v.submission))
  )
    return null;
  return value as ToolDraft;
}
const scalarId = (value: unknown) => {
  if (!value || typeof value !== "object" || !("kind" in value)) return "";
  return value.kind === "artifact" && "artifact_id" in value
    ? String(value.artifact_id)
    : String(value.kind);
};
function ScalarPicker({
  value,
  onChange,
  artifacts,
  label,
}: {
  value: unknown;
  onChange: (value: ScalarInput) => void;
  artifacts: Artifact[];
  label: string;
}) {
  const selected = scalarId(value);
  return (
    <select
      aria-label={label}
      value={selected}
      onChange={(event) =>
        onChange(
          event.target.value === "stored_bytes"
            ? { kind: "stored_bytes" }
            : event.target.value === "origin_width"
              ? { kind: "origin_width" }
              : { kind: "artifact", artifact_id: event.target.value },
        )
      }
    >
      {!selected && <option value="">选择输入字段</option>}
      <option value="stored_bytes">存储对象大小（字节）</option>
      <option value="origin_width">首条来源记录的原始观察宽度（px）</option>
      {selected &&
        !artifacts.some((a) => a.id === selected) &&
        !["stored_bytes", "origin_width"].includes(selected) && (
          <option value={selected}>
            已保存的成果 · {selected.slice(0, 8)}
          </option>
        )}
      {artifacts.map((a) => (
        <option key={a.id} value={a.id}>
          {a.name} · {a.id.slice(0, 8)} · {a.count} 项
        </option>
      ))}
    </select>
  );
}
export default function BasicToolPanel(context: ModuleContext) {
  const { client, projectId, inputOptions } = context;
  const cache = useQueryClient();
  const draft = useDraft(client, projectId, "core.tools", initial, decode);
  const value = draft.value;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const applied = useRef<number | null>(null);
  const operators = useQuery({
    queryKey: ["operators", client.connection.instance_id],
    queryFn: ({ signal }) => client.tools.operators(signal),
    select: (data) => ({
      ...data,
      items: data.items.filter(
        (o) =>
          !["danbooru.metarecall", "danbooru.metarecall_v2"].includes(o.id),
      ),
    }),
  });
  const artifacts = useQuery({
    queryKey: ["project", projectId, "artifacts", "inputs"],
    queryFn: ({ signal }) =>
      client.tools.artifacts(projectId, { limit: 128, signal }),
  });
  const scalarArtifacts =
    artifacts.data?.items.filter(
      (a) => a.kind === "scalar_columns" && a.state === "ready",
    ) ?? [];
  const operator = operators.data?.items.find(
    (o) =>
      o.id === value.operatorId &&
      o.version === value.operatorVersion &&
      o.parameters_version === value.parametersVersion,
  );
  function chooseOperator(operator: OperatorDescriptor) {
    draft.controller.set((v) => ({
      ...v,
      operatorId: operator.id,
      operatorVersion: operator.version,
      parametersVersion: operator.parameters_version,
      parameters: Object.fromEntries(
        operator.parameters.map((p) => [p.id, p.default_value]),
      ),
      submission: null,
    }));
  }
  function applyRun(run: OperatorRun) {
    const supported = operators.data?.items.find(
      (item) =>
        item.id === run.operator_id &&
        item.version === run.operator_version &&
        item.parameters_version === run.parameters_version,
    );
    if (
      !supported ||
      !run.parameters ||
      typeof run.parameters !== "object" ||
      Array.isArray(run.parameters)
    ) {
      setError("这份参数的格式或工具版本暂不兼容。");
      return;
    }
    draft.controller.set((old) => ({
      ...old,
      operatorId: run.operator_id,
      operatorVersion: run.operator_version,
      parametersVersion: run.parameters_version,
      parameters: run.parameters as Record<string, unknown>,
      submission: null,
    }));
    setNotice("已载入参数，输入范围仍以当前配置为准。确认后再启动计算。");
    context.management?.showProperties();
  }
  useEffect(() => {
    if (
      !draft.editable ||
      !context.invocation ||
      applied.current === context.invocation.sequence ||
      !operators.data
    )
      return;
    const args = context.invocation.args;
    if (args.reuseRun) {
      try {
        applyRun(JSON.parse(args.reuseRun) as OperatorRun);
      } catch {
        setError("历史参数无法读取，原配置已保留。");
      }
      applied.current = context.invocation.sequence;
      return;
    }
    const target = operators.data.items.find((o) => o.id === args.operatorId);
    if (target)
      draft.controller.set((v) => ({
        ...v,
        operatorId: target.id,
        operatorVersion: target.version,
        parametersVersion: target.parameters_version,
        parameters: {
          ...Object.fromEntries(
            target.parameters.map((p) => [p.id, p.default_value]),
          ),
          ...(args.artifactId
            ? { input: { kind: "artifact", artifact_id: args.artifactId } }
            : {}),
        },
        submission: null,
      }));
    applied.current = context.invocation.sequence;
  }, [draft.controller, draft.editable, context.invocation, operators.data]);
  useEffect(() => {
    if (
      draft.editable &&
      !value.scope &&
      !value.scopeId &&
      inputOptions.length
    ) {
      const option =
        inputOptions.find((o) => o.value === context.defaultInput) ??
        inputOptions[0];
      if (option)
        draft.controller.set((v) => ({
          ...v,
          scope: option.scope,
          scopeId: option.value,
        }));
    }
  }, [
    draft.controller,
    draft.editable,
    value.scope,
    value.scopeId,
    inputOptions,
    context.defaultInput,
  ]);
  const current = inputOptions.find((o) => o.value === value.scopeId);
  const scopeCheck = useQuery({
    queryKey: [
      "project",
      projectId,
      "scope-validity",
      value.scope,
      current?.scope,
    ],
    queryFn: ({ signal }) =>
      client.tools.validateScope(projectId, value.scope!, signal),
    enabled: !!value.scope && draft.editable,
    retry: false,
  });
  const stale =
    !!value.scope &&
    (scopeCheck.isError ||
      (!!current &&
        JSON.stringify(current.scope) !== JSON.stringify(value.scope)));
  const options =
    value.scope && !current
      ? [
          {
            value: value.scopeId,
            label: "已保存的输入范围",
            scope: value.scope,
            count: null,
          },
          ...inputOptions,
        ]
      : inputOptions;
  function parameter(id: string, next: unknown) {
    draft.controller.set((v) => ({
      ...v,
      parameters: { ...v.parameters, [id]: next },
      submission: null,
    }));
  }
  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!operator || !value.scope || stale) return;
    setPending(true);
    setError("");
    try {
      const run = {
        operator_id: value.operatorId,
        operator_version: value.operatorVersion,
        parameters_version: value.parametersVersion,
        parameters: value.parameters,
      };
      const signature = JSON.stringify({ run, scope: value.scope });
      const key =
        value.submission?.signature === signature
          ? value.submission.key
          : crypto.randomUUID();
      draft.controller.set((v) => ({ ...v, submission: { key, signature } }));
      await draft.controller.flush();
      const job = await client.tools.submit(projectId, {
        run,
        scope: value.scope,
        idempotency_key: key,
        delay_ms: 0,
      });
      draft.controller.set((v) => ({
        ...v,
        submission: null,
        lastJob: job.id,
      }));
      await cache.invalidateQueries({
        queryKey: ["project", projectId, "jobs"],
      });
      context.onJob(job);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setPending(false);
    }
  }
  return (
    <section className="tool-view" aria-label="基础工具">
      <div className="content-bar">
        <Calculator size={16} />
        <strong>基础工具</strong>
        <span className="grow" />
        <span className="subtle">选择输入并设置计算参数</span>
      </div>
      <DraftStatus controller={draft.controller} quiet />
      {notice && (
        <p className="tool-notice" role="status">
          {notice}
        </p>
      )}
      <div className="basic-presets">
        <PresetControls
          client={client}
          projectId={projectId}
          run={{
            operator_id: value.operatorId,
            operator_version: value.operatorVersion,
            parameters_version: value.parametersVersion,
            parameters: value.parameters,
          }}
          onApply={applyRun}
          disabled={!draft.editable || pending || !operator}
        />
      </div>
      <form onSubmit={(event) => void submit(event)}>
        <fieldset disabled={!draft.editable || pending}>
          <Field label="工具">
            <select
              aria-label="计算算子"
              value={operator ? operator.id : "unavailable"}
              onChange={(e) => {
                const next = operators.data?.items.find(
                  (o) => o.id === e.target.value,
                );
                if (next) chooseOperator(next);
              }}
            >
              {!operator && (
                <option value="unavailable">
                  所需算子版本尚不可用 · {value.operatorId} v
                  {value.operatorVersion}
                </option>
              )}
              {operators.data?.items.map((o) => (
                <option key={o.id + o.version} value={o.id}>
                  {o.name} · v{o.version}
                </option>
              ))}
            </select>
          </Field>
          <ScopePicker
            options={options}
            value={value.scopeId}
            onChange={(id) => {
              const option = inputOptions.find((o) => o.value === id);
              if (option)
                draft.controller.set((v) => ({
                  ...v,
                  scope: option.scope,
                  scopeId: id,
                  submission: null,
                }));
            }}
          />
          {value.scope?.target.kind === "selection" && (
            <details className="tool-input-details">
              <summary>输入范围说明</summary>
              <p className="tool-hint">
                已使用选择版本 {value.scope.target.revision}
                。选择变化后，可重新使用当前范围。
              </p>
            </details>
          )}
          {stale && (
            <div className="tool-notice" role="alert">
              已保存的输入范围已变化或暂不可用，请重新选择范围。
              {current && (
                <Button
                  type="button"
                  onClick={() =>
                    draft.controller.set((v) => ({
                      ...v,
                      scope: current.scope,
                      submission: null,
                    }))
                  }
                >
                  <RotateCw size={12} />
                  重新绑定当前范围
                </Button>
              )}
            </div>
          )}
          {operator?.parameters.map((p) => (
            <Field key={p.id} label={p.name}>
              {p.value_type === "scalar_input" ? (
                <ScalarPicker
                  label={p.name}
                  value={value.parameters[p.id]}
                  artifacts={scalarArtifacts}
                  onChange={(v) => parameter(p.id, v)}
                />
              ) : p.value_type === "scalar_inputs" ? (
                <div className="field-list">
                  {(Array.isArray(value.parameters[p.id])
                    ? (value.parameters[p.id] as unknown[])
                    : []
                  ).map((input, index) => (
                    <div key={index}>
                      <ScalarPicker
                        label={p.name + " " + (index + 1)}
                        value={input}
                        artifacts={scalarArtifacts}
                        onChange={(next) =>
                          parameter(
                            p.id,
                            (value.parameters[p.id] as unknown[]).map(
                              (old, i) => (i === index ? next : old),
                            ),
                          )
                        }
                      />
                      <Button
                        type="button"
                        onClick={() =>
                          parameter(
                            p.id,
                            (value.parameters[p.id] as unknown[]).filter(
                              (_, i) => i !== index,
                            ),
                          )
                        }
                      >
                        移除
                      </Button>
                    </div>
                  ))}
                  <Button
                    type="button"
                    disabled={
                      Array.isArray(value.parameters[p.id]) &&
                      (value.parameters[p.id] as unknown[]).length >= 8
                    }
                    onClick={() =>
                      parameter(p.id, [
                        ...(Array.isArray(value.parameters[p.id])
                          ? (value.parameters[p.id] as unknown[])
                          : []),
                        { kind: "stored_bytes" },
                      ])
                    }
                  >
                    添加固定字段
                  </Button>
                </div>
              ) : (
                <input
                  aria-label={p.name}
                  value={
                    typeof value.parameters[p.id] === "string"
                      ? (value.parameters[p.id] as string)
                      : ""
                  }
                  onChange={(e) => parameter(p.id, e.target.value)}
                  maxLength={32}
                  required={p.required}
                />
              )}
            </Field>
          ))}
          {operator && (
            <div className="tool-capabilities">
              <span>固定成员与字段值</span>
              {operator.capabilities.cancel && <span>可取消</span>}
              {operator.capabilities.checkpoint && <span>检查点恢复</span>}
              {operator.capabilities.item_failures && <span>单项失败明细</span>}
              <span>{operator.outputs.length} 类成果</span>
            </div>
          )}
          <p className="tool-hint">
            {operator?.parameters.some((p) => p.id === "multiplier")
              ? "标量计算：输入值 × 乘数 + 加数。"
              : ""}
            缺失、失败和未计算状态保留；字段值与观察依据随任务固定。
          </p>
          {scopeCheck.error && (
            <ErrorDetails compact error={scopeCheck.error} />
          )}
          <Button
            type="submit"
            className="primary"
            disabled={
              !operator ||
              !value.scope ||
              stale ||
              scopeCheck.isPending ||
              current?.count === 0
            }
          >
            <Play size={14} />
            {pending ? "正在提交…" : "提交任务"}
          </Button>
        </fieldset>
      </form>
      {(error || operators.error || artifacts.error) && (
        <ErrorDetails error={error || operators.error || artifacts.error} />
      )}
      {value.lastJob && (
        <p className="tool-hint">
          上次已提交任务：{value.lastJob.slice(0, 8)}
          。参数编辑保存在当前草稿中。
        </p>
      )}
    </section>
  );
}
