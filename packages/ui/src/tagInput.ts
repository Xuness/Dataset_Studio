// Canonical tags use literal U+0020. Quoted JSON strings express embedded
// whitespace and separators without relying on a single-line input's normalization.
export function validSourceTag(value: string, literal = false) {
  return (
    !!value &&
    new TextEncoder().encode(value).length <= 256 &&
    (literal || !value.includes(" ")) &&
    ![...value].some((c) => {
      const n = c.codePointAt(0)!;
      return (
        (n < 32 || (n >= 127 && n <= 159)) &&
        ![9, 10, 11, 12, 13, 133].includes(n)
      );
    })
  );
}
export function parseTagInput(text: string): {
  values: string[];
  error: string;
} {
  const values: string[] = [];
  let i = 0;
  while (i < text.length) {
    if (/[ ,，]/u.test(text[i] ?? "")) {
      i++;
      continue;
    }
    const start = i;
    if (text[i] === '"') {
      let closed = false;
      for (i++; i < text.length; i++) {
        if (text[i] === "\\") {
          i++;
          continue;
        }
        if (text[i] === '"') {
          i++;
          closed = true;
          break;
        }
      }
      if (!closed) return { values: [], error: "精确标签的双引号尚未闭合。" };
      try {
        values.push(JSON.parse(text.slice(start, i)) as string);
      } catch {
        return {
          values: [],
          error: "精确标签请使用 JSON 字符串转义，例如 \\\\t 或 \\\\n。",
        };
      }
      if (i < text.length && !/[ ,，]/u.test(text[i] ?? ""))
        return { values: [], error: "标签之间请用普通空格或逗号分隔。" };
    } else {
      while (i < text.length && !/[ ,，]/u.test(text[i] ?? "")) i++;
      values.push(text.slice(start, i));
    }
  }
  const unique = [...new Set(values)];
  if (unique.length > 64)
    return { values: unique, error: "每组最多 64 个标签。" };
  if (unique.some((value) => !validSourceTag(value)))
    return {
      values: unique,
      error: "标签不能为空、含普通空格或超过 256 字节。",
    };
  return { values: unique, error: "" };
}
export const tagList = (text: string) => parseTagInput(text).values;
export const formatTagList = (values: string[]) =>
  values
    .map((value) => (/[\s,，"\\]/u.test(value) ? JSON.stringify(value) : value))
    .join(" ");
export const displayTag = (tag: string) =>
  /[\s]/u.test(tag) ? JSON.stringify(tag) : tag;
