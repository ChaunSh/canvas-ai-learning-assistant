/**
 * Tauri event subscriptions.
 *
 * All long-running work reports progress through events instead of polling, so
 * the UI stays responsive while parsing, indexing or streaming.
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  CanvasConnection,
  CanvasDownloadEvent,
  ChatStreamEvent,
  IndexProgress,
  SyncProgressEvent,
} from "@/types/api";

export const EVENTS = {
  chatStream: "ai://stream",
  indexProgress: "materials://index",
  syncProgress: "canvas://sync",
  /** Emitted when the login watcher verifies a Canvas session by itself. */
  canvasConnection: "canvas://connection",
  /** Emitted while course files are being downloaded. */
  canvasDownload: "canvas://download",
} as const;

export function onChatStream(handler: (event: ChatStreamEvent) => void): Promise<UnlistenFn> {
  return listen<ChatStreamEvent>(EVENTS.chatStream, (message) => handler(message.payload));
}

export function onIndexProgress(handler: (event: IndexProgress) => void): Promise<UnlistenFn> {
  return listen<IndexProgress>(EVENTS.indexProgress, (message) => handler(message.payload));
}

export function onSyncProgress(handler: (event: SyncProgressEvent) => void): Promise<UnlistenFn> {
  return listen<SyncProgressEvent>(EVENTS.syncProgress, (message) => handler(message.payload));
}

export function onCanvasConnection(
  handler: (connection: CanvasConnection) => void,
): Promise<UnlistenFn> {
  return listen<CanvasConnection>(EVENTS.canvasConnection, (message) => handler(message.payload));
}

export function onCanvasDownload(
  handler: (event: CanvasDownloadEvent) => void,
): Promise<UnlistenFn> {
  return listen<CanvasDownloadEvent>(EVENTS.canvasDownload, (message) => handler(message.payload));
}
