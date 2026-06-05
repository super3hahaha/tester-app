// Shared date logic for Batch Reply config + execution pages.
// The preset is stored; the actual date range is computed at fetch time
// so configs stay "fresh" without users having to re-save daily.

export type DatePreset =
  | "sinceLastWorkday"
  | "yesterday"
  | "today"
  | "7d"
  | "custom";

export interface DateRange {
  fromDate: string; // YYYY-MM-DD
  toDate: string;   // YYYY-MM-DD
}

export function toIso(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export function daysAgo(n: number, base: Date = new Date()): Date {
  const d = new Date(base);
  d.setDate(d.getDate() - n);
  return d;
}

// "Last workday before today" — covers the gap since the user was last at work:
//   Tue–Fri → yesterday
//   Mon     → last Friday
//   Sat     → Friday (yesterday)
//   Sun     → Friday (2 days ago)
export function lastWorkdayBefore(today: Date = new Date()): Date {
  const dow = today.getDay(); // 0=Sun, 1=Mon, ..., 6=Sat
  let back = 1;
  if (dow === 1) back = 3;
  else if (dow === 0) back = 2;
  return daysAgo(back, today);
}

export function computeRange(
  preset: DatePreset,
  custom: { fromDate: string; toDate: string },
  now: Date = new Date(),
): DateRange {
  switch (preset) {
    case "sinceLastWorkday":
      return { fromDate: toIso(lastWorkdayBefore(now)), toDate: toIso(now) };
    case "yesterday": {
      const y = toIso(daysAgo(1, now));
      return { fromDate: y, toDate: y };
    }
    case "today": {
      const t = toIso(now);
      return { fromDate: t, toDate: t };
    }
    case "7d":
      return { fromDate: toIso(daysAgo(6, now)), toDate: toIso(now) };
    case "custom":
    default:
      return { fromDate: custom.fromDate, toDate: custom.toDate };
  }
}

export const PRESET_LABELS: Record<DatePreset, string> = {
  sinceLastWorkday: "自上一个工作日",
  yesterday: "昨天",
  today: "今天",
  "7d": "近 7 天",
  custom: "自定义",
};
