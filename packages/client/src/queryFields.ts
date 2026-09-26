import type { Schema } from "@studio/contracts";

type Directory = Schema["FieldDirectory"];
export function commonQueryFields(directories: Directory[]) {
  const first = directories[0];
  return {
    direct_query:
      directories.length > 0 && directories.every((d) => d.direct_query),
    fields: (first?.fields ?? []).flatMap((field) => {
      const peers = directories.map((d) =>
        d.fields.find((f) => f.id === field.id),
      );
      if (
        peers.some(
          (f) =>
            !f || f.field_type !== field.field_type || f.unit !== field.unit,
        )
      )
        return [];
      const operators = field.operators.filter((op) =>
        peers.every((f) => f!.operators.includes(op)),
      );
      return operators.length
        ? [{ ...field, operators, sortable: peers.every((f) => f!.sortable) }]
        : [];
    }),
    orders: (first?.orders ?? []).filter((order) =>
      directories.every((d) => d.orders.includes(order)),
    ),
    observation_rules: (first?.observation_rules ?? []).filter((rule) =>
      directories.every((d) => d.observation_rules.includes(rule)),
    ),
    max_conditions: first
      ? Math.min(...directories.map((d) => d.max_conditions))
      : 0,
  };
}
