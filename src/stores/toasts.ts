import { create } from "zustand";

export type ToastKind = "success" | "error" | "info";

export interface Toast {
  id: string;
  kind: ToastKind;
  message: string;
  detail?: string;
}

interface ToastState {
  toasts: Toast[];
  push: (kind: ToastKind, message: string, detail?: string) => void;
  dismiss: (id: string) => void;
}

let counter = 0;

export const useToasts = create<ToastState>((set, get) => ({
  toasts: [],
  push: (kind, message, detail) => {
    const id = `toast-${++counter}`;
    set({ toasts: [...get().toasts, { id, kind, message, detail }] });
    const ttl = kind === "error" ? 8000 : 4000;
    window.setTimeout(() => get().dismiss(id), ttl);
  },
  dismiss: (id) => set({ toasts: get().toasts.filter((toast) => toast.id !== id) }),
}));

/** Imperative helper for non-component code. */
export function toast(kind: ToastKind, message: string, detail?: string) {
  useToasts.getState().push(kind, message, detail);
}
