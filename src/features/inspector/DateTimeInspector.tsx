import { useState, useMemo, useCallback } from "react";
import { Clock, Globe, Calendar, Copy, Check, AlertCircle } from "lucide-react";
import type { ColumnInfo } from "../../ipc/generated";
import {
  parseDateTime,
  formatInTimezone,
  formatRelativeTime,
  getCalendarBreakdown,
  COMMON_TIMEZONES,
} from "./datetime-utils";

export interface DateTimeInspectorProps {
  value: string | null;
  column?: Pick<ColumnInfo, "name" | "databaseType" | "kind">;
  sessionTimezone?: string;
  compact?: boolean;
}

export function DateTimeInspector({
  value,
  column: _column,
  sessionTimezone = "America/Los_Angeles",
  compact = false,
}: DateTimeInspectorProps) {
  const [selectedSessionTz, setSelectedSessionTz] = useState<string>(sessionTimezone);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  // 1. Parse date
  const parsed = useMemo(() => parseDateTime(value), [value]);

  // 2. Detect local browser timezone
  const localTimezone = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
    } catch {
      return "UTC";
    }
  }, []);

  // 3. Formatted string representations
  const localFormatted = useMemo(
    () => (parsed ? formatInTimezone(parsed.date, localTimezone) : ""),
    [parsed, localTimezone],
  );

  const utcFormatted = useMemo(
    () => (parsed ? `${parsed.date.toISOString().replace("T", " ")}` : ""),
    [parsed],
  );

  const sessionFormatted = useMemo(
    () => (parsed ? formatInTimezone(parsed.date, selectedSessionTz) : ""),
    [parsed, selectedSessionTz],
  );

  const relativeAge = useMemo(() => (parsed ? formatRelativeTime(parsed.epochMs) : ""), [parsed]);

  const calendar = useMemo(() => (parsed ? getCalendarBreakdown(parsed.date) : null), [parsed]);

  const handleCopy = useCallback(async (text: string, key: string) => {
    await navigator.clipboard.writeText(text);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 1500);
  }, []);

  if (value === null || !parsed) {
    return (
      <div className="inspector-placeholder" data-testid="datetime-inspector">
        {value === null ? (
          "NULL"
        ) : (
          <div className="datetime-error-banner" role="alert">
            <AlertCircle size={13} />
            <span>Unrecognized date/timestamp format: "{value}"</span>
          </div>
        )}
      </div>
    );
  }

  const epochSec = Math.floor(parsed.epochMs / 1000);

  return (
    <div
      className={`datetime-inspector ${compact ? "compact" : ""}`}
      data-testid="datetime-inspector"
    >
      {/* Header: Relative Age Badge */}
      <div className="datetime-header-badge">
        <Clock size={13} />
        <span className="datetime-relative-pill">{relativeAge}</span>
        {!parsed.hasTimezone && (
          <span
            className="datetime-ntz-pill"
            title="Column lacks timezone offset (e.g. TIMESTAMP_NTZ); rendered as UTC wall-clock"
          >
            No TZ (Wall-clock)
          </span>
        )}
      </div>

      {/* Simultaneous Timezone Breakdowns */}
      <div className="datetime-tz-cards">
        {/* 1. Local Browser Timezone */}
        <div className="datetime-card">
          <div className="datetime-card-header">
            <span className="datetime-card-title">
              <Globe size={11} /> Local Browser ({localTimezone})
            </span>
            <button
              type="button"
              className="icon-button micro"
              title="Copy Local Time"
              aria-label="Copy local time"
              onClick={() => handleCopy(localFormatted, "local")}
            >
              {copiedKey === "local" ? <Check size={11} /> : <Copy size={11} />}
            </button>
          </div>
          <div className="datetime-card-value">{localFormatted}</div>
        </div>

        {/* 2. UTC */}
        <div className="datetime-card">
          <div className="datetime-card-header">
            <span className="datetime-card-title">
              <Globe size={11} /> UTC (Universal Time)
            </span>
            <button
              type="button"
              className="icon-button micro"
              title="Copy UTC / ISO 8601"
              aria-label="Copy UTC time"
              onClick={() => handleCopy(utcFormatted, "utc")}
            >
              {copiedKey === "utc" ? <Check size={11} /> : <Copy size={11} />}
            </button>
          </div>
          <div className="datetime-card-value">{utcFormatted}</div>
        </div>

        {/* 3. Session / Warehouse Timezone */}
        <div className="datetime-card">
          <div className="datetime-card-header">
            <span className="datetime-card-title">
              <Globe size={11} /> Session TZ
            </span>
            <select
              className="datetime-tz-select"
              value={selectedSessionTz}
              onChange={(e) => setSelectedSessionTz(e.target.value)}
              aria-label="Select session timezone"
            >
              {COMMON_TIMEZONES.map((tz) => (
                <option key={tz.id} value={tz.id}>
                  {tz.id} ({tz.offset})
                </option>
              ))}
            </select>
            <button
              type="button"
              className="icon-button micro"
              title="Copy Session Time"
              aria-label="Copy session time"
              onClick={() => handleCopy(sessionFormatted, "session")}
            >
              {copiedKey === "session" ? <Check size={11} /> : <Copy size={11} />}
            </button>
          </div>
          <div className="datetime-card-value">{sessionFormatted}</div>
        </div>
      </div>

      {/* Epoch Breakdowns */}
      <div className="datetime-section">
        <div className="datetime-section-title">UNIX EPOCH</div>
        <div className="datetime-epoch-grid">
          <div className="datetime-epoch-row">
            <span className="datetime-epoch-label">Milliseconds:</span>
            <code className="datetime-epoch-val">{parsed.epochMs}</code>
            <button
              type="button"
              className="icon-button micro"
              title="Copy Milliseconds"
              aria-label="Copy epoch milliseconds"
              onClick={() => handleCopy(String(parsed.epochMs), "ms")}
            >
              {copiedKey === "ms" ? <Check size={10} /> : <Copy size={10} />}
            </button>
          </div>

          <div className="datetime-epoch-row">
            <span className="datetime-epoch-label">Seconds:</span>
            <code className="datetime-epoch-val">{epochSec}</code>
            <button
              type="button"
              className="icon-button micro"
              title="Copy Seconds"
              aria-label="Copy epoch seconds"
              onClick={() => handleCopy(String(epochSec), "sec")}
            >
              {copiedKey === "sec" ? <Check size={10} /> : <Copy size={10} />}
            </button>
          </div>
        </div>
      </div>

      {/* Calendar Breakdown */}
      {calendar && (
        <div className="datetime-section">
          <div className="datetime-section-title">
            <Calendar size={11} /> CALENDAR BREAKDOWN
          </div>
          <div className="datetime-calendar-grid">
            <div className="datetime-calendar-cell">
              <span className="cal-label">Day of Week</span>
              <span className="cal-val">
                {calendar.dayOfWeekName} ({calendar.isoDayOfWeek})
              </span>
            </div>
            <div className="datetime-calendar-cell">
              <span className="cal-label">Day of Year</span>
              <span className="cal-val">
                Day {calendar.dayOfYear} of {calendar.totalDaysInYear}
              </span>
            </div>
            <div className="datetime-calendar-cell">
              <span className="cal-label">ISO Week</span>
              <span className="cal-val">Week {calendar.isoWeekNumber}</span>
            </div>
            <div className="datetime-calendar-cell">
              <span className="cal-label">Quarter</span>
              <span className="cal-val">{calendar.quarter}</span>
            </div>
            <div className="datetime-calendar-cell">
              <span className="cal-label">Leap Year</span>
              <span className="cal-val">{calendar.isLeapYear ? "Yes" : "No"}</span>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
