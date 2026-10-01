export type DiffLine = {
  kind: "context" | "add" | "remove";
  text: string;
  oldLine?: number;
  newLine?: number;
};
export function fileDiff(before: Buffer | null, after: Buffer | null) {
  function decode(value: Buffer | null) {
    if (!value) return "";
    const text = new TextDecoder("utf-8", { fatal: true }).decode(value);
    if (text.includes("\0")) throw new Error("binary");
    return text;
  }
  let oldText: string, newText: string;
  try {
    oldText = decode(before);
    newText = decode(after);
  } catch {
    return {
      lines: [] as DiffLine[],
      message:
        "Binary change. Download the before and after versions to compare.",
      coarse: false,
    };
  }
  if (oldText === newText)
    return {
      lines: [] as DiffLine[],
      message: "No content changes.",
      coarse: false,
    };
  if (oldText.length + newText.length > 1_000_000)
    return {
      lines: [] as DiffLine[],
      message:
        "Large text change. Full versions are retained; download them to compare.",
      coarse: true,
    };
  const oldLines = oldText ? oldText.match(/[^\n]*\n|[^\n]+$/g)! : [];
  const newLines = newText ? newText.match(/[^\n]*\n|[^\n]+$/g)! : [];
  if (oldLines.length + newLines.length > 30000)
    return {
      lines: [] as DiffLine[],
      message:
        "Too many lines for an inline diff. Download the full versions to compare.",
      coarse: true,
    };
  let prefix = 0,
    suffix = 0;
  while (
    prefix < oldLines.length &&
    prefix < newLines.length &&
    oldLines[prefix] === newLines[prefix]
  )
    prefix++;
  while (
    suffix < oldLines.length - prefix &&
    suffix < newLines.length - prefix &&
    oldLines[oldLines.length - suffix - 1] ===
      newLines[newLines.length - suffix - 1]
  )
    suffix++;
  const a = oldLines.slice(prefix, oldLines.length - suffix),
    b = newLines.slice(prefix, newLines.length - suffix);
  const lines: DiffLine[] = [];
  let oldLine = Math.max(0, prefix - 3) + 1,
    newLine = oldLine;
  const emit = (kind: DiffLine["kind"], text: string) =>
    lines.push({
      kind,
      text,
      ...(kind !== "add" ? { oldLine: oldLine++ } : {}),
      ...(kind !== "remove" ? { newLine: newLine++ } : {}),
    });
  for (const line of oldLines.slice(Math.max(0, prefix - 3), prefix))
    emit("context", line);
  const coarse = (a.length + 1) * (b.length + 1) > 2_000_000;
  if (coarse) {
    a.forEach((line) => emit("remove", line));
    b.forEach((line) => emit("add", line));
  } else {
    const width = b.length + 1,
      table = new Uint32Array((a.length + 1) * width);
    for (let i = a.length - 1; i >= 0; i--)
      for (let j = b.length - 1; j >= 0; j--)
        table[i * width + j] =
          a[i] === b[j]
            ? 1 + table[(i + 1) * width + j + 1]
            : Math.max(table[(i + 1) * width + j], table[i * width + j + 1]);
    let i = 0,
      j = 0;
    while (i < a.length || j < b.length) {
      if (i < a.length && j < b.length && a[i] === b[j]) {
        emit("context", a[i++]);
        j++;
      } else if (
        i < a.length &&
        (j === b.length ||
          table[(i + 1) * width + j] >= table[i * width + j + 1])
      )
        emit("remove", a[i++]);
      else emit("add", b[j++]);
    }
  }
  oldLines
    .slice(
      oldLines.length - suffix,
      oldLines.length - suffix + Math.min(suffix, 3),
    )
    .forEach((line) => emit("context", line));
  return {
    lines,
    coarse,
    message: coarse ? "Large changed region shown as a replacement." : "",
  };
}
