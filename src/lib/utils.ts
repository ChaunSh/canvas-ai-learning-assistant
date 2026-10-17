import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import { format, formatDistanceToNow, isValid, parseISO } from "date-fns";
import { enUS, zhCN } from "date-fns/locale";

import type { Locale } from "@/i18n";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value.toFixed(value >= 10 || index === 0 ? 0 : 1)} ${units[index]}`;
}

function dateLocale(locale: Locale) {
  return locale === "zh" ? zhCN : enUS;
}

export function parseDate(value?: string | null): Date | null {
  if (!value) return null;
  const parsed = parseISO(value);
  return isValid(parsed) ? parsed : null;
}

export function formatDate(value: string | null | undefined, locale: Locale = "zh"): string {
  const date = parseDate(value);
  if (!date) return "—";
  return format(date, locale === "zh" ? "yyyy年M月d日" : "MMM d, yyyy", {
    locale: dateLocale(locale),
  });
}

export function formatDateTime(value: string | null | undefined, locale: Locale = "zh"): string {
  const date = parseDate(value);
  if (!date) return "—";
  return format(date, locale === "zh" ? "M月d日 HH:mm" : "MMM d, HH:mm", {
    locale: dateLocale(locale),
  });
}

export function formatTime(value: string | null | undefined): string {
  const date = parseDate(value);
  if (!date) return "—";
  return format(date, "HH:mm");
}

export function formatRelative(value: string | null | undefined, locale: Locale = "zh"): string {
  const date = parseDate(value);
  if (!date) return "—";
  return formatDistanceToNow(date, { addSuffix: true, locale: dateLocale(locale) });
}

/** `YYYY-MM-DD` for `<input type="date">` in local time. */
export function toDateInput(value?: string | Date | null): string {
  const date = value instanceof Date ? value : parseDate(value ?? null) ?? new Date();
  return format(date, "yyyy-MM-dd");
}

/** `YYYY-MM-DDTHH:mm` for `<input type="datetime-local">` in local time. */
export function toDateTimeInput(value?: string | Date | null): string {
  const date = value instanceof Date ? value : parseDate(value ?? null) ?? new Date();
  return format(date, "yyyy-MM-dd'T'HH:mm");
}

/** Convert a `datetime-local` value to an ISO-8601 UTC instant. */
export function fromDateTimeInput(value: string): string {
  const date = new Date(value);
  return isValid(date) ? date.toISOString() : new Date().toISOString();
}

export function fromDateInput(value: string): string {
  const date = new Date(`${value}T00:00:00`);
  return isValid(date) ? date.toISOString() : new Date().toISOString();
}

export function percent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, Math.round(value * 100)));
}

export function truncate(text: string, max = 120): string {
  if (text.length <= max) return text;
  return `${text.slice(0, max)}…`;
}

export function isOnline(): boolean {
  return typeof navigator === "undefined" ? true : navigator.onLine;
}

/** Stable, human-friendly file size limit for client-side hints. */
export const MAX_IMPORT_BYTES = 300 * 1024 * 1024;

/** Extract a displayable message from any thrown value. */
export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    const value = (error as { message?: unknown }).message;
    if (typeof value === "string") return value;
  }
  return "未知错误";
}
