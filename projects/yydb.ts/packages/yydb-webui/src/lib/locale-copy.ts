/** Shared locale helpers for `#locales` message refresh after locale transitions. */

export type LocaleId = "zh-hans" | string;

export function readLocaleId(preferred?: string | null): LocaleId {
    if (preferred) return preferred;
    if (typeof document !== "undefined") {
        return document.documentElement.getAttribute("data-locale") || "zh-hans";
    }
    const hint = typeof globalThis !== "undefined" ? (globalThis as any).__vmzLocaleIdHint : undefined;
    return hint || "zh-hans";
}

export function withLocaleHint<T>(localeId: LocaleId, fn: () => T): T {
    const g = typeof globalThis !== "undefined" ? (globalThis as any) : null;
    const prev = g ? g.__vmzLocaleIdHint : undefined;
    if (g) g.__vmzLocaleIdHint = localeId;
    try {
        return fn();
    } finally {
        if (g) {
            if (prev === undefined) delete g.__vmzLocaleIdHint;
            else g.__vmzLocaleIdHint = prev;
        }
    }
}

export function watchDocumentLocale(onChange: (localeId: string) => void): () => void {
    if (typeof document === "undefined" || typeof MutationObserver === "undefined") {
        return () => {};
    }
    const el = document.documentElement;
    let last = el.getAttribute("data-locale") || "";
    const obs = new MutationObserver(() => {
        const next = el.getAttribute("data-locale") || "";
        if (next === last) return;
        last = next;
        if (next) onChange(next);
    });
    obs.observe(el, { attributes: true, attributeFilter: ["data-locale"] });
    return () => obs.disconnect();
}
