export const MAX_SELECTION_STAT_CELLS = 10_000;

const EXACT_DECIMAL_PATTERN = /^(-?)(0|[1-9]\d*)(?:\.(\d+))?$/;
const MAX_DECIMAL_DIGITS = 38;

export interface GridSelectionStats {
  cellCount: number;
  nonNullCount: number;
  distinctCount: number;
  numericCount: number;
  sum: string | null;
  avg: string | null;
  min: string | null;
  max: string | null;
  overLimit: boolean;
}

interface ScaledDecimal {
  raw: string;
  mantissa: bigint;
  scale: number;
}

function pow10(exp: number): bigint {
  return 10n ** BigInt(exp);
}

function parseExactDecimal(value: string): ScaledDecimal | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const match = EXACT_DECIMAL_PATTERN.exec(trimmed);
  if (!match) return null;
  const [, sign, intDigits, fracDigits = ""] = match;
  if (intDigits.length > MAX_DECIMAL_DIGITS || fracDigits.length > MAX_DECIMAL_DIGITS) {
    return null;
  }
  const scale = fracDigits.length;
  const unsigned = BigInt(intDigits + fracDigits);
  const mantissa = sign === "-" && unsigned !== 0n ? -unsigned : unsigned;
  return { raw: trimmed, mantissa, scale };
}

function compareScaled(a: ScaledDecimal, b: ScaledDecimal): number {
  const targetScale = Math.max(a.scale, b.scale);
  const left = a.mantissa * pow10(targetScale - a.scale);
  const right = b.mantissa * pow10(targetScale - b.scale);
  return left < right ? -1 : left > right ? 1 : 0;
}

function formatScaled(mantissa: bigint, scale: number, trimTrailingZeros: boolean): string {
  if (mantissa === 0n) {
    return scale > 0 && !trimTrailingZeros ? `0.${"0".repeat(scale)}` : "0";
  }
  const negative = mantissa < 0n;
  const abs = (negative ? -mantissa : mantissa).toString();
  if (scale === 0) {
    return `${negative ? "-" : ""}${abs}`;
  }
  const padded = abs.padStart(scale + 1, "0");
  const intPart = padded.slice(0, padded.length - scale);
  let fracPart = padded.slice(padded.length - scale);
  if (trimTrailingZeros) {
    fracPart = fracPart.replace(/0+$/, "");
  }
  return fracPart.length > 0
    ? `${negative ? "-" : ""}${intPart}.${fracPart}`
    : `${negative ? "-" : ""}${intPart}`;
}

function divideRounded(
  sumMantissa: bigint,
  sumScale: number,
  count: bigint,
  outScale: number,
): string {
  const extraScale = outScale - sumScale;
  const scaledDividend = extraScale >= 0 ? sumMantissa * pow10(extraScale) : sumMantissa;
  const effectiveDivisor = extraScale >= 0 ? count : count * pow10(-extraScale);
  const negative = scaledDividend < 0n;
  const absDividend = negative ? -scaledDividend : scaledDividend;
  const quotient = absDividend / effectiveDivisor;
  const remainder = absDividend % effectiveDivisor;
  const roundedAbs = remainder * 2n >= effectiveDivisor ? quotient + 1n : quotient;
  const signed = negative && roundedAbs !== 0n ? -roundedAbs : roundedAbs;
  return formatScaled(signed, outScale, true);
}

/**
 * Computes lossless summary aggregates across a multi-cell grid selection.
 * Returns `null` when fewer than 2 cells are selected so single-cell clicks stay quiet.
 */
export function computeSelectionStats(
  cells: readonly (string | null)[],
  maxCells: number = MAX_SELECTION_STAT_CELLS,
): GridSelectionStats | null {
  const cellCount = cells.length;
  if (cellCount < 2) return null;
  if (cellCount > maxCells) {
    return {
      cellCount,
      nonNullCount: 0,
      distinctCount: 0,
      numericCount: 0,
      sum: null,
      avg: null,
      min: null,
      max: null,
      overLimit: true,
    };
  }

  let nonNullCount = 0;
  const distinct = new Set<string>();
  let numericCount = 0;
  let sumMantissa = 0n;
  let sumScale = 0;
  let minVal: ScaledDecimal | null = null;
  let maxVal: ScaledDecimal | null = null;

  for (let i = 0; i < cellCount; i++) {
    const cell = cells[i];
    if (cell === null) continue;
    nonNullCount++;
    distinct.add(cell);

    const parsed = parseExactDecimal(cell);
    if (!parsed) continue;
    if (numericCount === 0) {
      sumMantissa = parsed.mantissa;
      sumScale = parsed.scale;
      minVal = parsed;
      maxVal = parsed;
    } else {
      if (parsed.scale > sumScale) {
        sumMantissa *= pow10(parsed.scale - sumScale);
        sumScale = parsed.scale;
        sumMantissa += parsed.mantissa;
      } else if (parsed.scale < sumScale) {
        sumMantissa += parsed.mantissa * pow10(sumScale - parsed.scale);
      } else {
        sumMantissa += parsed.mantissa;
      }
      if (compareScaled(parsed, minVal!) < 0) minVal = parsed;
      if (compareScaled(parsed, maxVal!) > 0) maxVal = parsed;
    }
    numericCount++;
  }

  if (numericCount === 0) {
    return {
      cellCount,
      nonNullCount,
      distinctCount: distinct.size,
      numericCount: 0,
      sum: null,
      avg: null,
      min: null,
      max: null,
      overLimit: false,
    };
  }

  const sum = formatScaled(sumMantissa, sumScale, false);
  const avgScale = Math.min(MAX_DECIMAL_DIGITS, Math.max(sumScale, 4));
  const avg = divideRounded(sumMantissa, sumScale, BigInt(numericCount), avgScale);

  return {
    cellCount,
    nonNullCount,
    distinctCount: distinct.size,
    numericCount,
    sum,
    avg,
    min: minVal!.raw,
    max: maxVal!.raw,
    overLimit: false,
  };
}
