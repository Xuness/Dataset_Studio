import { RankingV2Config } from "./RankingV2Config.js";
import { v2Defaults } from "./v2.js";
import type { V2Parameters } from "./v2.js";
import type { FormEvent, ReactNode } from "react";
import type { RankingParameters } from "@studio/contracts";
import { Button, Field } from "@studio/ui";
import type { ModuleScopeOption } from "@studio/ui";
import { ScopePicker } from "../scopes/ScopePicker.js";
import { defaults } from "./types.js";

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <details className="ranking-config-section" open>
      <summary>{title}</summary>
      <div className="ranking-section-content">{children}</div>
    </details>
  );
}

export function RankingConfig({
  id,
  parameters: p,
  onChange,
  options,
  scopeId,
  onScope,
  disabled,
  onSubmit,
  scopeMessage,
  onRebind,
  v2Saved,
}: {
  id: string;
  parameters: RankingParameters;
  v2Saved?: V2Parameters | null;
  onChange: (p: RankingParameters) => void;
  options: ModuleScopeOption[];
  scopeId: string;
  onScope: (id: string) => void;
  disabled: boolean;
  onSubmit: (event: FormEvent) => void;
  scopeMessage: string;
  onRebind: () => void;
}) {
  function field<K extends keyof RankingParameters>(
    key: K,
    value: RankingParameters[K],
  ) {
    onChange({ ...p, [key]: value });
  }
  return (
    <form id={id} className="ranking-config" onSubmit={onSubmit}>
      <fieldset disabled={disabled} className="ranking-config-fields">
        <Section title="输入与输出">
          <Field label="筛选方案">
            <select
              aria-label="筛选方案"
              value={p.v2 ? "v2" : "v1"}
              onChange={(e) =>
                field(
                  "v2",
                  e.target.value === "v2" ? (v2Saved ?? v2Defaults()) : null,
                )
              }
            >
              <option value="v1">MetaRecall v1 · 旧方案</option>
              <option value="v2">MetaRecall v2 · 元数据多阶段</option>
            </select>
          </Field>
          <ScopePicker options={options} value={scopeId} onChange={onScope} />
          <Field label="同图重复帖热度">
            <select
              aria-label="同图重复帖热度"
              value={p.duplicate_heat ?? "legacy"}
              onChange={(e) =>
                field(
                  "duplicate_heat",
                  e.target.value === "legacy"
                    ? null
                    : (e.target.value as "highest" | "sum"),
                )
              }
            >
              <option value="highest">取较高记录（默认）</option>
              <option value="sum">不同帖子求和</option>
              <option value="legacy">旧规则：单一代表记录</option>
            </select>
          </Field>
          <p className="ranking-hint">
            新规则按最新元数据确定分级；热度只在同一图片的不同帖子间处理，同帖历史不重复累计。求和表示帖子互动合计，无法去重同一用户的重复投票。
          </p>
          {scopeMessage && (
            <div className="ranking-notice">
              {scopeMessage}
              <Button type="button" onClick={onRebind}>
                刷新并重新绑定范围
              </Button>
            </div>
          )}
          <div className="ranking-rating-row">
            <span className="ranking-field-label">参与分级</span>
            <div
              className="ranking-ratings"
              role="group"
              aria-label="参与计算的分级"
            >
              {["g", "s", "q", "e"].map((r) => (
                <label key={r}>
                  <input
                    type="checkbox"
                    checked={p.ratings.includes(r)}
                    onChange={(e) =>
                      field(
                        "ratings",
                        (e.target.checked
                          ? [...p.ratings, r]
                          : p.ratings.filter((v) => v !== r)
                        ).sort(),
                      )
                    }
                  />
                  {r.toUpperCase()}
                </label>
              ))}
            </div>
          </div>
          <fieldset className="ranking-mode">
            <legend>本次输出</legend>
            <label>
              <input
                type="radio"
                name="ranking-mode"
                checked={p.mode === "rank"}
                onChange={() => field("mode", "rank")}
              />
              仅计算排名
            </label>
            <label>
              <input
                type="radio"
                name="ranking-mode"
                checked={p.mode === "select"}
                onChange={() => field("mode", "select")}
              />
              按名额生成候选集
            </label>
          </fieldset>
          <p className="ranking-hint ranking-indented">
            各分级独立统计。提交后固定输入范围与元数据。
          </p>
        </Section>

        <Section title="用途条件">
          <div className="ranking-purpose-row">
            <label className="ranking-check">
              <input
                type="checkbox"
                checked={p.minimum_stored_side != null}
                onChange={(e) =>
                  field("minimum_stored_side", e.target.checked ? 768 : null)
                }
              />
              启用成品最短边门槛
            </label>
            {p.minimum_stored_side != null && (
              <Field label="最短边">
                <input
                  aria-label="成品最短边（px）"
                  type="number"
                  min={1}
                  max={65535}
                  step={1}
                  value={p.minimum_stored_side}
                  onChange={(e) =>
                    field("minimum_stored_side", Number(e.target.value))
                  }
                />
                <span>px</span>
              </Field>
            )}
          </div>
          <label className="ranking-check">
            <input
              type="checkbox"
              checked={p.exclude_banned}
              onChange={(e) => field("exclude_banned", e.target.checked)}
            />
            排除源站封禁条目
          </label>
          <p className="ranking-hint">
            {p.minimum_stored_side == null
              ? "成品尺寸不参与本次评分。"
              : "严格检查成品尺寸；尺寸未知的条目保留为待核验。"}
          </p>
        </Section>

        <Section title="评分与补救">
          <div className="ranking-feature-grid">
            {(
              [
                ["time_enabled", "时间条件化补救"],
                ["artist_enabled", "画师先验"],
                ["votes_enabled", "有限负票扣分"],
                ["damage_enabled", "明确技术损伤标签"],
              ] as const
            ).map(([key, label]) => (
              <label className="ranking-check" key={key}>
                <input
                  type="checkbox"
                  checked={p[key]}
                  onChange={(e) => field(key, e.target.checked)}
                />
                {key === "time_enabled" && p.v2 ? "时间与年代参考" : label}
              </label>
            ))}
          </div>
          <p className="ranking-hint">
            时间或比较群体不足时 C=G；画师先验关闭或支持不足时 A=0。
          </p>
        </Section>

        {p.v2 && (
          <RankingV2Config
            value={p.v2}
            selecting={p.mode === "select"}
            onChange={(value) => field("v2", value)}
          />
        )}
        {p.v2 && !p.time_enabled && (
          <p className="ranking-hint">
            时间与年代参考已关闭，本次融合使用直算榜；手动年代偏好仍按独立配置生效。
          </p>
        )}
        {p.mode === "select" && !p.v2 && (
          <Section title="通道名额">
            <div className="ranking-quota-fields">
              {["主通道", "补救通道", "随机审计"].map((label, i) => (
                <Field label={label} key={label}>
                  <input
                    aria-label={label + "（%）"}
                    type="number"
                    min={0}
                    max={100}
                    step={0.1}
                    value={(p.quotas[i] ?? 0) / 10}
                    onChange={(e) => {
                      const quotas = [...p.quotas];
                      quotas[i] = Math.round(Number(e.target.value) * 10);
                      field("quotas", quotas);
                    }}
                  />
                  <span>%</span>
                </Field>
              ))}
            </div>
            <p className="ranking-hint">
              占各分级合格候选池的{" "}
              {(p.quotas.reduce((a, b) => a + b, 0) / 10).toFixed(1)}
              %，依次分配三个互不重复的通道。
            </p>
          </Section>
        )}

        <details className="ranking-config-section ranking-advanced">
          <summary>高级参数与公式</summary>
          <div className="ranking-section-content">
            <div className="ranking-parameter-grid">
              {(
                [
                  ["time_weight", "时间补救 α"],
                  ["artist_weight", "画师帮助 γ"],
                  ["vote_weight", "负票惩罚 β"],
                  ["damage_weight", "单类损伤"],
                ] as const
              )
                .filter(
                  ([key]) =>
                    !p.v2 || (key !== "time_weight" && key !== "vote_weight"),
                )
                .map(([key, label]) => (
                  <Field key={key} label={label}>
                    <input
                      aria-label={label}
                      type="number"
                      min={0}
                      max={1}
                      step={0.01}
                      value={p[key]}
                      onChange={(e) => field(key, Number(e.target.value))}
                    />
                  </Field>
                ))}
            </div>
            <Field label="邻域样本下限">
              <input
                aria-label="邻域最低有效数量"
                type="number"
                min={1}
                max={1000000}
                step={1}
                value={p.cohort_minimum}
                onChange={(e) =>
                  field("cohort_minimum", Number(e.target.value))
                }
              />
            </Field>
            <Field label="随机种子">
              <input
                aria-label="随机种子"
                type="text"
                value={p.seed}
                maxLength={128}
                onChange={(e) => field("seed", e.target.value)}
              />
            </Field>
            <div className="ranking-formula">
              {p.v2 ? (
                "直算 + 年代相对 → 总体百分位融合 → 类型调整 + 年代偏好"
              ) : (
                <>
                  S = 100 × clip[G + α × max(C − G, 0) − βV − T]
                  <br />R = 100 × clip[C + γA(1 − C) − βV − T]
                </>
              )}
            </div>
            <p className="ranking-hint">
              上传邻域依次扩展到 ±6、±12、±24
              个月，再使用两级合并帖龄桶。技术损伤总扣分上限为 8 分。
            </p>
          </div>
        </details>
        <footer className="ranking-config-footer">
          <span>{p.v2 ? "MetaRecall v2 · 元数据模式" : "MetaRecall v1"}</span>
          <Button
            type="button"
            onClick={() =>
              onChange({
                ...defaults,
                ...(p.v2 ? { v2: v2Defaults() } : {}),
                ratings: [...defaults.ratings],
                quotas: [...defaults.quotas],
              })
            }
          >
            恢复默认参数
          </Button>
        </footer>
      </fieldset>
    </form>
  );
}
