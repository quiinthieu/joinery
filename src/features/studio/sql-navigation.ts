import type { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import type { AppError, ErrorLocation } from "../../ipc/generated";

export interface StatementRange {
  from: number;
  to: number;
}

export interface ResolvedDocumentErrorRange {
  from: number;
  to: number;
  caret: number;
  line: number;
  column: number;
}

const isIgnorableStatementChild = (name: string) =>
  name === "Semi" || name === ";" || name.endsWith("Comment");

/**
 * Computes ordered non-empty SQL statement spans in UTF-16 code units, ignoring
 * leading/standalone comments, empty semicolons, and semicolons inside strings,
 * quoted identifiers, dollar-quoted strings, or comments.
 */
export function statementRanges(state: EditorState): StatementRange[] {
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (!tree) return [];
  const ranges: StatementRange[] = [];
  for (let node = tree.topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== "Statement") continue;
    let firstSqlChild: typeof node | null = null;
    for (let child = node.firstChild; child; child = child.nextSibling) {
      if (child.to <= child.from || isIgnorableStatementChild(child.name)) continue;
      if (!state.sliceDoc(child.from, child.to).trim()) continue;
      firstSqlChild = child;
      break;
    }
    if (!firstSqlChild) continue;
    ranges.push({ from: firstSqlChild.from, to: node.to });
  }
  return ranges;
}

/**
 * Finds the previous (`-1`) or next (`1`) SQL statement relative to the main
 * caret position (`state.selection.main.head`).
 */
export function adjacentStatement(state: EditorState, direction: -1 | 1): StatementRange | null {
  const ranges = statementRanges(state);
  if (ranges.length === 0) return null;
  const head = state.selection.main.head;
  const containingIndex = ranges.findIndex((range) => head >= range.from && head <= range.to);
  if (containingIndex >= 0) {
    return ranges[containingIndex + direction] ?? null;
  }
  if (direction === 1) {
    return ranges.find((range) => range.from > head) ?? null;
  }
  for (let i = ranges.length - 1; i >= 0; i -= 1) {
    if (ranges[i].from < head) return ranges[i];
  }
  return null;
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

function clampUtf16Boundary(text: string, pos: number, roundUp: boolean): number {
  const clamped = Math.max(0, Math.min(text.length, Number.isFinite(pos) ? Math.trunc(pos) : 0));
  if (clamped > 0 && clamped < text.length) {
    const prev = text.charCodeAt(clamped - 1);
    const curr = text.charCodeAt(clamped);
    if (isHighSurrogate(prev) && isLowSurrogate(curr)) {
      return roundUp ? clamped + 1 : clamped - 1;
    }
  }
  return clamped;
}

function stepForwardCodePoint(text: string, pos: number): number {
  if (pos >= text.length) return text.length;
  const curr = text.charCodeAt(pos);
  if (pos + 1 < text.length && isHighSurrogate(curr) && isLowSurrogate(text.charCodeAt(pos + 1))) {
    return pos + 2;
  }
  return pos + 1;
}

function stepBackwardCodePoint(text: string, pos: number): number {
  if (pos <= 0) return 0;
  const prev = text.charCodeAt(pos - 1);
  if (pos >= 2 && isLowSurrogate(prev) && isHighSurrogate(text.charCodeAt(pos - 2))) {
    return pos - 2;
  }
  return pos - 1;
}

function lineAndColumnAtUtf16(text: string, targetUtf16: number): { line: number; column: number } {
  const limit = clampUtf16Boundary(text, targetUtf16, false);
  let line = 1;
  let column = 1;
  let i = 0;
  while (i < limit) {
    const code = text.charCodeAt(i);
    if (code === 0x0a /* \n */) {
      line += 1;
      column = 1;
      i += 1;
    } else if (code === 0x0d /* \r */) {
      if (i + 1 < text.length && text.charCodeAt(i + 1) === 0x0a) {
        i += 1;
      } else {
        column += 1;
        i += 1;
      }
    } else if (i + 1 < limit && isHighSurrogate(code) && isLowSurrogate(text.charCodeAt(i + 1))) {
      column += 1;
      i += 2;
    } else {
      column += 1;
      i += 1;
    }
  }
  return { line, column };
}

/**
 * Translates a backend `ErrorLocation` (relative to the submitted SQL slice at
 * `offsetUtf16`) into safe UTF-16 document bounds and 1-based Unicode scalar
 * `(line, column)` coordinates suitable for CodeMirror decorations and jumping.
 */
export function resolveDocumentErrorRange(
  docText: string,
  location: ErrorLocation,
  offsetUtf16 = 0,
): ResolvedDocumentErrorRange {
  const base = Number.isFinite(offsetUtf16) ? Math.max(0, Math.trunc(offsetUtf16)) : 0;
  const caret = clampUtf16Boundary(docText, base + location.span.startUtf16, false);
  let from = caret;
  let to = clampUtf16Boundary(
    docText,
    Math.max(base + location.span.startUtf16, base + location.span.endUtf16),
    true,
  );
  if (docText.length > 0 && from === to) {
    if (from < docText.length && docText[from] !== "\n" && docText[from] !== "\r") {
      to = stepForwardCodePoint(docText, from);
    } else if (from > 0) {
      from = stepBackwardCodePoint(docText, from);
    } else {
      to = stepForwardCodePoint(docText, from);
    }
  }
  const { line, column } = lineAndColumnAtUtf16(docText, caret);
  return { from, to, caret, line, column };
}

export interface SqlExecutionOrigin {
  document: string;
  query: string;
  fromUtf16: number;
}

/** CodeMirror stores one UTF-16 newline unit per line break. */
export function editorDocument(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

/** Resolve only against the exact document and submitted slice that failed. */
export function resolveExecutionError(
  document: string,
  error?: AppError | null,
  origin?: SqlExecutionOrigin,
): ResolvedDocumentErrorRange | null {
  const span = error?.location?.span;
  if (
    !origin ||
    !span ||
    editorDocument(origin.document) !== editorDocument(document) ||
    !origin.query ||
    !Number.isSafeInteger(origin.fromUtf16) ||
    origin.fromUtf16 < 0 ||
    origin.document.slice(origin.fromUtf16, origin.fromUtf16 + origin.query.length) !==
      origin.query ||
    !Number.isSafeInteger(span.startUtf16) ||
    !Number.isSafeInteger(span.endUtf16) ||
    span.startUtf16 < 0 ||
    span.endUtf16 < span.startUtf16 ||
    span.endUtf16 > origin.query.length
  )
    return null;
  const rawStart = origin.fromUtf16 + span.startUtf16;
  const rawEnd = origin.fromUtf16 + span.endUtf16;
  const from = editorDocument(origin.document.slice(0, rawStart)).length;
  const to = editorDocument(origin.document.slice(0, rawEnd)).length;
  const resolved = resolveDocumentErrorRange(editorDocument(document), {
    ...error!.location!,
    span: { ...span, startUtf16: from, endUtf16: to },
  });
  return {
    ...resolved,
    from: Math.max(
      editorDocument(origin.document.slice(0, origin.fromUtf16)).length,
      resolved.from,
    ),
    to: Math.min(
      editorDocument(origin.document.slice(0, origin.fromUtf16 + origin.query.length)).length,
      resolved.to,
    ),
  };
}
