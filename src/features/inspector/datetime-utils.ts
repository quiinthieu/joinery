export interface DateTimeParseResult {
  date: Date;
  epochMs: number;
  nanos?: string;
  hasTimezone: boolean;
  originalUnit: "string" | "ms" | "s" | "us" | "ns";
}

export interface CalendarBreakdown {
  dayOfWeekName: string; // e.g. "Thursday"
  isoDayOfWeek: number; // 1 (Monday) to 7 (Sunday)
  dayOfYear: number; // 1 to 365/366
  totalDaysInYear: number;
  isoWeekNumber: number; // e.g. 45
  quarter: string; // "Q1", "Q2", "Q3", "Q4"
  isLeapYear: boolean;
}

export const COMMON_TIMEZONES = [
  { id: "UTC", label: "UTC (Coordinated Universal Time)", offset: "+00:00" },
  { id: "America/New_York", label: "New York (US Eastern)", offset: "-05:00" },
  { id: "America/Chicago", label: "Chicago (US Central)", offset: "-06:00" },
  { id: "America/Denver", label: "Denver (US Mountain)", offset: "-07:00" },
  {
    id: "America/Los_Angeles",
    label: "Los Angeles (US Pacific / Snowflake default)",
    offset: "-08:00",
  },
  { id: "Europe/London", label: "London (GMT / BST)", offset: "+00:00" },
  { id: "Europe/Berlin", label: "Frankfurt / Berlin (CET / CEST)", offset: "+01:00" },
  { id: "Europe/Paris", label: "Paris (CET / CEST)", offset: "+01:00" },
  { id: "Asia/Dubai", label: "Dubai (GST)", offset: "+04:00" },
  { id: "Asia/Kolkata", label: "Kolkata (IST)", offset: "+05:30" },
  { id: "Asia/Singapore", label: "Singapore (SGT)", offset: "+08:00" },
  { id: "Asia/Tokyo", label: "Tokyo (JST)", offset: "+09:00" },
  { id: "Australia/Sydney", label: "Sydney (AEST / AEDT)", offset: "+10:00" },
  { id: "Pacific/Auckland", label: "Auckland (NZST / NZDT)", offset: "+12:00" },
];

/**
 * Parses timestamps in ISO 8601, SQL dates, Snowflake TIMESTAMP_NTZ/LTZ/TZ, and Epoch numbers.
 */
export function parseDateTime(input: string | number | null): DateTimeParseResult | null {
  if (input === null || input === undefined) return null;

  if (typeof input === "number") {
    return parseEpochNumber(input);
  }

  const str = input.trim();
  if (!str) return null;

  // 1. Pure epoch number as string
  if (/^[+-]?\d+(?:\.\d+)?$/.test(str)) {
    const num = Number(str);
    if (Number.isFinite(num)) {
      return parseEpochNumber(num);
    }
  }

  // 2. ISO 8601 / SQL standard regex
  // Matches: 2026-11-05, 2026-11-05 15:30:00, 2026-11-05T15:30:00.123Z, 2026-11-05 15:30:00 +0000
  const isoRegex =
    /^(\d{4})-(\d{2})-(\d{2})(?:[T\s](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?)?(?:\s*(Z|[+-]\d{2}(?::?\d{2})?))?$/i;
  const match = str.match(isoRegex);

  if (match) {
    const [, y, m, d, hh = "00", mm = "00", ss = "00", frac = "0", tz] = match;
    const msFrac = Number((frac + "000").slice(0, 3));

    if (tz) {
      // Has explicit timezone
      const date = new Date(str.replace(" ", "T"));
      if (!Number.isNaN(date.getTime())) {
        return {
          date,
          epochMs: date.getTime(),
          nanos: frac,
          hasTimezone: true,
          originalUnit: "string",
        };
      }
    } else {
      // Snowflake TIMESTAMP_NTZ / plain datetime: interpret as UTC wall-clock
      const year = Number(y);
      const month = Number(m) - 1;
      const day = Number(d);
      const utcMs = Date.UTC(year, month, day, Number(hh), Number(mm), Number(ss), msFrac);
      const date = new Date(utcMs);
      if (year >= 0 && year < 100) {
        date.setUTCFullYear(year, month, day);
      }
      return {
        date,
        epochMs: date.getTime(),
        nanos: frac,
        hasTimezone: false,
        originalUnit: "string",
      };
    }
  }

  // 3. Fallback to native Date.parse
  const parsed = Date.parse(str);
  if (!Number.isNaN(parsed)) {
    return {
      date: new Date(parsed),
      epochMs: parsed,
      hasTimezone: true,
      originalUnit: "string",
    };
  }

  return null;
}

function parseEpochNumber(num: number): DateTimeParseResult {
  let ms = num;
  let unit: "ms" | "s" | "us" | "ns" = "ms";

  if (Math.abs(num) > 1e16) {
    ms = num / 1e6;
    unit = "ns";
  } else if (Math.abs(num) > 1e13) {
    ms = num / 1e3;
    unit = "us";
  } else if (Math.abs(num) > 1e11) {
    ms = num;
    unit = "ms";
  } else {
    ms = num * 1000;
    unit = "s";
  }

  return {
    date: new Date(ms),
    epochMs: Math.round(ms),
    hasTimezone: true,
    originalUnit: unit,
  };
}

/**
 * Formats a Date in a given IANA timezone with full offset and timezone name.
 */
export function formatInTimezone(date: Date, timeZone: string): string {
  if (!date || Number.isNaN(date.getTime())) {
    return "Invalid Date";
  }

  try {
    const formatter = new Intl.DateTimeFormat("en-CA", {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      fractionalSecondDigits: 3,
      hour12: false,
      timeZoneName: "shortOffset",
    });
    // e.g. "2026-11-05, 15:30:00.000 GMT+7" -> "2026-11-05 15:30:00.000 GMT+7"
    return formatter.format(date).replace(", ", " ");
  } catch {
    // If timezone string is invalid, fallback to UTC
    try {
      return date.toISOString().replace("T", " ");
    } catch {
      return "Invalid Date";
    }
  }
}

/**
 * Formats relative humanized age (e.g. "2 hours ago", "in 3 days").
 */
export function formatRelativeTime(targetMs: number, nowMs = Date.now()): string {
  const diffSec = Math.round((targetMs - nowMs) / 1000);
  const absSec = Math.abs(diffSec);

  if (absSec < 5) return "just now";

  const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

  if (absSec < 60) return rtf.format(diffSec, "second");
  const diffMin = Math.round(diffSec / 60);
  if (Math.abs(diffMin) < 60) return rtf.format(diffMin, "minute");
  const diffHour = Math.round(diffMin / 60);
  if (Math.abs(diffHour) < 24) return rtf.format(diffHour, "hour");
  const diffDay = Math.round(diffHour / 24);
  if (Math.abs(diffDay) < 30) return rtf.format(diffDay, "day");
  const diffMonth = Math.round(diffDay / 30);
  if (Math.abs(diffMonth) < 12) return rtf.format(diffMonth, "month");
  const diffYear = Math.round(diffDay / 365);
  return rtf.format(diffYear, "year");
}

/**
 * Calculates day of week, day of year, ISO week, and quarter.
 */
export function getCalendarBreakdown(date: Date): CalendarBreakdown {
  const year = date.getUTCFullYear();
  const month = date.getUTCMonth();

  const daysOfWeek = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
  const dayOfWeekName = daysOfWeek[date.getUTCDay()];
  const isoDayOfWeek = date.getUTCDay() === 0 ? 7 : date.getUTCDay();

  const startOfYear = Date.UTC(year, 0, 1);
  const diffMs = date.getTime() - startOfYear;
  const dayOfYear = Math.floor(diffMs / (24 * 3600 * 1000)) + 1;
  const isLeapYear = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  const totalDaysInYear = isLeapYear ? 366 : 365;

  // ISO Week Number
  const target = new Date(date.valueOf());
  const dayNr = (date.getUTCDay() + 6) % 7;
  target.setUTCDate(target.getUTCDate() - dayNr + 3);
  const firstThursday = target.valueOf();
  target.setUTCMonth(0, 1);
  if (target.getUTCDay() !== 4) {
    target.setUTCMonth(0, 1 + ((4 - target.getUTCDay() + 7) % 7));
  }
  const isoWeekNumber = 1 + Math.ceil((firstThursday - target.valueOf()) / (7 * 24 * 3600 * 1000));

  const quarter = `Q${Math.floor(month / 3) + 1}`;

  return {
    dayOfWeekName,
    isoDayOfWeek,
    dayOfYear,
    totalDaysInYear,
    isoWeekNumber,
    quarter,
    isLeapYear,
  };
}
