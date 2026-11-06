import { describe, expect, it } from "vitest";
import {
  parseDateTime,
  formatInTimezone,
  formatRelativeTime,
  getCalendarBreakdown,
} from "./datetime-utils";

describe("datetime-utils", () => {
  it("parses ISO 8601 UTC timestamp string", () => {
    const res = parseDateTime("2026-11-05T15:30:00Z");
    expect(res).not.toBeNull();
    if (res) {
      expect(res.epochMs).toBe(1793892600000);
      expect(res.hasTimezone).toBe(true);
      expect(res.date.toISOString()).toBe("2026-11-05T15:30:00.000Z");
    }
  });

  it("parses SQL timestamp string with no timezone as UTC wall-clock", () => {
    const res = parseDateTime("2026-11-05 15:30:00");
    expect(res).not.toBeNull();
    if (res) {
      expect(res.epochMs).toBe(1793892600000);
      expect(res.hasTimezone).toBe(false);
    }
  });

  it("parses Unix epoch numbers and seconds/milliseconds", () => {
    const resMs = parseDateTime(1793892600000);
    expect(resMs?.epochMs).toBe(1793892600000);
    expect(resMs?.originalUnit).toBe("ms");

    const resSec = parseDateTime(1793892600);
    expect(resSec?.epochMs).toBe(1793892600000);
    expect(resSec?.originalUnit).toBe("s");
  });

  it("formats timestamp in various timezones", () => {
    const date = new Date("2026-11-05T15:30:00Z");
    const utcStr = formatInTimezone(date, "UTC");
    expect(utcStr).toContain("2026-11-05");
    expect(utcStr).toContain("15:30:00");

    const nyStr = formatInTimezone(date, "America/New_York");
    expect(nyStr).toContain("2026-11-05");
    expect(nyStr).toContain("10:30:00"); // UTC-5
  });

  it("computes relative time accurately", () => {
    const now = 1793892600000;
    expect(formatRelativeTime(now, now)).toBe("just now");
    expect(formatRelativeTime(now - 7200000, now)).toBe("2 hours ago");
    expect(formatRelativeTime(now + 86400000 * 3, now)).toBe("in 3 days");
  });

  it("calculates calendar breakdown accurately", () => {
    const date = new Date("2026-11-05T15:30:00Z"); // November 5, 2026 is Thursday
    const cal = getCalendarBreakdown(date);
    expect(cal.dayOfWeekName).toBe("Thursday");
    expect(cal.isoDayOfWeek).toBe(4);
    expect(cal.quarter).toBe("Q4");
    expect(cal.isLeapYear).toBe(false);
    expect(cal.totalDaysInYear).toBe(365);
    expect(cal.dayOfYear).toBe(309);
    expect(cal.isoWeekNumber).toBe(45);
  });
});
