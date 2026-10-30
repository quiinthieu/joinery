import type { ColumnInfo } from "../../ipc/generated";
import type { DirtyCell, DraftInsertCell } from "../../data/dirty-store";

export const MAX_PASTE_CELLS = 5000;

export type PasteRowTarget =
  | {
      kind: "existing";
      resultId: string;
      row: number;
      deleted?: boolean;
      readOriginal(columnId: string): string | null | undefined;
    }
  | {
      kind: "draftInsert";
      tabId: string;
      insertId: string;
    }
  | { kind: "unloaded" }
  | { kind: "outOfBounds" };

export type PasteOutcome =
  { ok: true; cells: DirtyCell[]; insertCells: DraftInsertCell[] } | { ok: false; error: string };

/**
 * Parses TSV clipboard text (supporting quoted fields with embedded tabs/newlines
 * and spreadsheet trailing newlines) into a 2D cell matrix.
 */
export function parseTsvClipboard(raw: string): string[][] {
  if (raw === "") return [[""]];
  const normalized = raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let i = 0;
  let inQuotes = false;
  let cellQuoted = false;

  while (i < normalized.length) {
    const ch = normalized[i];
    if (inQuotes) {
      if (ch === '"') {
        if (normalized[i + 1] === '"') {
          cell += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i++;
        continue;
      }
      cell += ch;
      i++;
      continue;
    }
    if (ch === '"' && cell === "" && !cellQuoted) {
      inQuotes = true;
      cellQuoted = true;
      i++;
      continue;
    }
    if (ch === "\t") {
      row.push(cell);
      cell = "";
      cellQuoted = false;
      i++;
      continue;
    }
    if (ch === "\n") {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
      cellQuoted = false;
      i++;
      continue;
    }
    cell += ch;
    i++;
  }
  const endsWithNewline = !inQuotes && normalized.endsWith("\n");
  if (!endsWithNewline) {
    row.push(cell);
    rows.push(row);
  }
  return rows.length ? rows : [[""]];
}

const INTEGER_PATTERN = /^[+-]?\d+$/;
const DECIMAL_PATTERN = /^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/;

export function coercePastedValue(
  raw: string,
  column: ColumnInfo,
): { ok: true; value: string | null } | { ok: false; reason: string } {
  const isEmptyNonText = raw === "" && column.kind !== "text" && column.kind !== "unknown";
  if (raw === "\\N" || isEmptyNonText) {
    if (!column.nullable) {
      return { ok: false, reason: "does not allow NULL" };
    }
    return { ok: true, value: null };
  }

  switch (column.kind) {
    case "integer": {
      const trimmed = raw.trim();
      if (!INTEGER_PATTERN.test(trimmed)) {
        return { ok: false, reason: "expects an exact integer" };
      }
      return { ok: true, value: trimmed };
    }
    case "decimal": {
      const trimmed = raw.trim();
      if (!DECIMAL_PATTERN.test(trimmed)) {
        return { ok: false, reason: "expects a valid decimal number" };
      }
      return { ok: true, value: trimmed };
    }
    case "boolean": {
      const lower = raw.trim().toLowerCase();
      if (lower === "true" || lower === "t" || lower === "1") {
        return { ok: true, value: "true" };
      }
      if (lower === "false" || lower === "f" || lower === "0") {
        return { ok: true, value: "false" };
      }
      return { ok: false, reason: "expects a boolean (true/false, t/f, 1/0)" };
    }
    case "json": {
      try {
        JSON.parse(raw);
        return { ok: true, value: raw };
      } catch {
        return { ok: false, reason: "expects valid JSON" };
      }
    }
    case "temporal": {
      const trimmed = raw.trim();
      if (!trimmed) {
        return { ok: false, reason: "expects a date/time value" };
      }
      return { ok: true, value: trimmed };
    }
    case "binary":
      return { ok: false, reason: "is binary and cannot be pasted as text" };
    case "text":
    case "unknown":
      return { ok: true, value: raw };
  }
}

export function validatePasteMatrix(options: {
  anchor: readonly [number, number];
  values: readonly (readonly string[])[];
  visibleColumns: readonly ColumnInfo[];
  tableEditable: boolean;
  resolveRow(visualRow: number): PasteRowTarget;
}): PasteOutcome {
  const { anchor, values, visibleColumns, tableEditable, resolveRow } = options;
  if (!tableEditable) {
    return { ok: false, error: "This table view is read-only." };
  }
  const [startCol, startRow] = anchor;
  let cellCount = 0;
  for (const row of values) cellCount += row.length;
  if (cellCount === 0) {
    return { ok: false, error: "Clipboard is empty." };
  }
  if (cellCount > MAX_PASTE_CELLS) {
    return {
      ok: false,
      error: `Paste is limited to ${MAX_PASTE_CELLS.toLocaleString()} cells at once.`,
    };
  }

  const cells: DirtyCell[] = [];
  const insertCells: DraftInsertCell[] = [];
  for (let dy = 0; dy < values.length; dy++) {
    const line = values[dy];
    const visualRow = startRow + dy;
    const rowTarget = resolveRow(visualRow);
    if (rowTarget.kind === "outOfBounds") {
      return {
        ok: false,
        error: `Paste exceeds available rows at row ${visualRow + 1}.`,
      };
    }
    if (rowTarget.kind === "unloaded") {
      return {
        ok: false,
        error: `Row ${visualRow + 1} is not loaded yet. Scroll to load target rows before pasting.`,
      };
    }
    if (rowTarget.kind === "existing" && rowTarget.deleted) {
      return {
        ok: false,
        error: `Row ${visualRow + 1} is marked for deletion.`,
      };
    }
    for (let dx = 0; dx < line.length; dx++) {
      const visualCol = startCol + dx;
      if (visualCol < 0 || visualCol >= visibleColumns.length) {
        return {
          ok: false,
          error: `Paste exceeds visible columns at row ${visualRow + 1}, column ${visualCol + 1}.`,
        };
      }
      const column = visibleColumns[visualCol];
      if (rowTarget.kind === "draftInsert") {
        if (!column.insertable) {
          return {
            ok: false,
            error: `Row ${visualRow + 1}, column "${column.name}" cannot be set on insert.`,
          };
        }
        const rawCell = line[dx];
        if (
          rawCell === "" &&
          column.hasDefault &&
          column.kind !== "text" &&
          column.kind !== "unknown"
        ) {
          insertCells.push({
            tabId: rowTarget.tabId,
            insertId: rowTarget.insertId,
            column: column.id,
            value: undefined,
          });
          continue;
        }
        const coerced = coercePastedValue(rawCell, column);
        if (!coerced.ok) {
          return {
            ok: false,
            error: `Row ${visualRow + 1}, column "${column.name}" ${coerced.reason}.`,
          };
        }
        insertCells.push({
          tabId: rowTarget.tabId,
          insertId: rowTarget.insertId,
          column: column.id,
          value: coerced.value,
        });
        continue;
      }
      if (!column.editable) {
        return {
          ok: false,
          error: `Row ${visualRow + 1}, column "${column.name}" is read-only.`,
        };
      }
      const original = rowTarget.readOriginal(column.id);
      if (original === undefined) {
        return {
          ok: false,
          error: `Row ${visualRow + 1} is not loaded yet. Scroll to load target rows before pasting.`,
        };
      }
      const coerced = coercePastedValue(line[dx], column);
      if (!coerced.ok) {
        return {
          ok: false,
          error: `Row ${visualRow + 1}, column "${column.name}" ${coerced.reason}.`,
        };
      }
      cells.push({
        resultId: rowTarget.resultId,
        row: rowTarget.row,
        column: column.id,
        original,
        value: coerced.value,
      });
    }
  }
  return { ok: true, cells, insertCells };
}
