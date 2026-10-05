import { v2Defaults } from "./v2.js";
import type { V2Parameters } from "./v2.js";
import { Button, Field, ResetButton } from "@studio/ui";

const ratings = ["g", "s", "q", "e"] as const;
const profileFields = [
  ["time_up", "时间上浮"],
  ["time_down", "时间下调"],
  ["vote_weight", "负票权重"],
  ["era_weight", "年代榜权重"],
] as const;
const base = v2Defaults();

export function RankingV2Config({
  value: p,
  onChange,
  selecting,
}: {
  value: V2Parameters;
  onChange: (p: V2Parameters) => void;
  selecting: boolean;
}) {
  function field<K extends keyof V2Parameters>(key: K, value: V2Parameters[K]) {
    onChange({ ...p, [key]: value });
  }
  const reset = (
    key:
      | "feather_days"
      | "minimum_effective"
      | "comic_penalty"
      | "keep_per_mille"
      | "direct_rescue"
      | "era_rescue"
      | "audit",
  ) => (p[key] !== base[key] ? () => field(key, base[key]) : undefined);
  const profileOf = (r: string) => p.profiles[r] ?? base.profiles[r]!;
  const numberField = (
    key: "feather_days" | "minimum_effective" | "comic_penalty",
    label: string,
    min: number,
    max: number,
    step = 1,
  ) => (
    <Field key={key} label={label} onReset={reset(key)}>
      <input
        aria-label={label}
        type="number"
        value={p[key]}
        min={min}
        max={max}
        step={step}
        onChange={(e) => field(key, Number(e.target.value))}
      />
    </Field>
  );
  return (
    <div className="ranking-v2-config">
      <details className="ranking-config-section" open>
        <summary>v2 · 分级公式与融合</summary>
        <div className="ranking-section-content">
          <p className="ranking-hint">
            首版只使用元数据。年代相对榜尚未经过 Bridge 视觉校准，融合权重为 0
            时使用直算榜、为 1 时使用年代相对榜。
          </p>
          <table className="property-matrix">
            <thead>
              <tr>
                <th scope="col">分级参数</th>
                {ratings.map((r) => (
                  <th scope="col" key={r}>
                    {r.toUpperCase()}
                  </th>
                ))}
                <td />
              </tr>
            </thead>
            <tbody>
              {profileFields.map(([key, label]) => {
                const changed = ratings.some(
                  (r) => profileOf(r)[key] !== base.profiles[r]![key],
                );
                return (
                  <tr key={key} className={changed ? "field-changed" : ""}>
                    <th scope="row">{label}</th>
                    {ratings.map((r) => (
                      <td key={r}>
                        <input
                          aria-label={r.toUpperCase() + " " + label}
                          type="number"
                          min={0}
                          max={1}
                          step={0.01}
                          value={profileOf(r)[key]}
                          onChange={(e) =>
                            field("profiles", {
                              ...p.profiles,
                              [r]: {
                                ...profileOf(r),
                                [key]: Number(e.target.value),
                              },
                            })
                          }
                        />
                      </td>
                    ))}
                    <td className="property-matrix-reset">
                      {changed && (
                        <ResetButton
                          label={label}
                          onReset={() =>
                            field("profiles", {
                              ...p.profiles,
                              ...Object.fromEntries(
                                ratings.map((r) => [
                                  r,
                                  {
                                    ...profileOf(r),
                                    [key]: base.profiles[r]![key],
                                  },
                                ]),
                              ),
                            })
                          }
                        />
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <p className="ranking-hint">
            直算分使用 G、C
            的有限双向修正；两榜在同一分级总体内统一为百分位，再计算融合分。
          </p>
        </div>
      </details>
      <details className="ranking-config-section" open>
        <summary>时间羽化与类型保护</summary>
        <div className="ranking-section-content">
          <div className="ranking-parameter-grid">
            {numberField("feather_days", "跨年羽化半宽（天）", 0, 180)}
            {numberField("minimum_effective", "年代有效样本下限", 1, 10000000)}
            {numberField("comic_penalty", "扩展漫画标签软降分", 0, 10, 0.5)}
          </div>
          <p className="ranking-hint">
            90 天半宽对应约 180 天的平滑过渡带，0
            为硬边界对照。帖龄使用宽分组；时间或样本不足时回退到已有条件化表现。
          </p>
          <p className="ranking-hint">
            默认关闭未经视觉确认的类型降分，可手动设置软降分幅度。comic
            单标签不扣分。多视角、特写、设定展示和 2–4
            格线索受到保护；长条、多页、多组或 5–6
            格线索只作有上限的软降分。这些是元数据线索，不是视觉确认的漫画分类；设为
            0 可关闭类型降分。
          </p>
        </div>
      </details>
      {selecting && (
        <details className="ranking-config-section" open>
          <summary>v2 · 保留预算与补救</summary>
          <div className="ranking-section-content">
            <div className="ranking-quota-fields">
              {(
                [
                  ["keep_per_mille", "合格候选保留比例"],
                  ["direct_rescue", "直榜独有补救"],
                  ["era_rescue", "年代榜独有补救"],
                  ["audit", "剩余池随机审计"],
                ] as const
              ).map(([key, label]) => (
                <Field key={key} label={label} onReset={reset(key)}>
                  <input
                    aria-label={label + "（%）"}
                    type="number"
                    min={0}
                    max={100}
                    step={0.1}
                    value={p[key] / 10}
                    onChange={(e) =>
                      field(key, Math.round(Number(e.target.value) * 10))
                    }
                  />
                  <span>%</span>
                </Field>
              ))}
            </div>
            <p className="ranking-hint">
              保留比例以各分级合格候选为分母；补救和审计比例以最终保留预算为分母。独有补救不足时名额回流主榜，全部入口合计不增加预算。
            </p>
          </div>
        </details>
      )}
      <details className="ranking-config-section" open>
        <summary>年代偏好与目标占比</summary>
        <div className="ranking-section-content">
          <p className="ranking-hint">
            年代偏好独立于质量基准分，边界同样羽化。目标占比仅在生成候选集时生效，每个分级按自身保留预算分别执行；未设置目标的年代共用剩余名额。
          </p>
          {p.eras.map((era, index) => (
            <fieldset className="ranking-v2-era" key={index}>
              <legend>年代区间 {index + 1}</legend>
              <div className="ranking-parameter-grid">
                {(
                  [
                    ["from_year", "起始年"],
                    ["through_year", "结束年"],
                    ["bonus", "偏好分"],
                  ] as const
                ).map(([key, label]) => (
                  <Field label={label} key={key}>
                    <input
                      aria-label={"年代 " + (index + 1) + " " + label}
                      type="number"
                      min={key === "bonus" ? -20 : 1900}
                      max={key === "bonus" ? 20 : 2200}
                      step={key === "bonus" ? 0.5 : 1}
                      value={era[key]}
                      onChange={(e) =>
                        field(
                          "eras",
                          p.eras.map((v, i) =>
                            i === index
                              ? { ...v, [key]: Number(e.target.value) }
                              : v,
                          ),
                        )
                      }
                    />
                  </Field>
                ))}
              </div>
              <label className="ranking-check">
                <input
                  type="checkbox"
                  checked={era.target_share != null}
                  onChange={(e) =>
                    field(
                      "eras",
                      p.eras.map((v, i) =>
                        i === index
                          ? {
                              ...v,
                              target_share: e.target.checked ? 100 : null,
                            }
                          : v,
                      ),
                    )
                  }
                />
                设置目标占比
              </label>
              {era.target_share != null && (
                <Field label="保留预算占比">
                  <input
                    aria-label={"年代 " + (index + 1) + " 目标占比（%）"}
                    type="number"
                    min={0}
                    max={100}
                    step={0.1}
                    value={era.target_share / 10}
                    onChange={(e) =>
                      field(
                        "eras",
                        p.eras.map((v, i) =>
                          i === index
                            ? {
                                ...v,
                                target_share: Math.round(
                                  Number(e.target.value) * 10,
                                ),
                              }
                            : v,
                        ),
                      )
                    }
                  />
                  <span>%</span>
                </Field>
              )}
              <Button
                type="button"
                onClick={() =>
                  field(
                    "eras",
                    p.eras.filter((_, i) => i !== index),
                  )
                }
              >
                移除区间
              </Button>
            </fieldset>
          ))}
          <Button
            type="button"
            disabled={p.eras.length >= 64}
            onClick={() =>
              field("eras", [
                ...p.eras,
                {
                  from_year: p.eras.length
                    ? Math.max(...p.eras.map((e) => e.through_year)) + 1
                    : 2024,
                  through_year: p.eras.length
                    ? Math.max(...p.eras.map((e) => e.through_year)) + 1
                    : 2026,
                  bonus: 0,
                  target_share: null,
                },
              ])
            }
          >
            添加年代区间
          </Button>
          <label className="ranking-check">
            <input
              type="checkbox"
              checked={!p.strict_era_targets}
              onChange={(e) => field("strict_era_targets", !e.target.checked)}
            />
            年代候选不足时允许名额回流
          </label>
          <p className="ranking-hint">
            关闭回流时，不可满足的年代目标会使任务返回明确原因。配额按实际上传年份计数，每张图片只占一份名额。
          </p>
        </div>
      </details>
    </div>
  );
}
