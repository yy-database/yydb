import { STROKE_ICONS } from './icons-stroke.js';
import { TEXT_BADGE_ICONS } from './icons-text-badge.js';
export { DEFAULT_STROKE_WIDTH } from './icon-types.js';
export const REGISTRY = {
    ...STROKE_ICONS,
    ...TEXT_BADGE_ICONS,
};
export const LEGACY_ICON_ALIASES = {
    check: 'action.check',
    close: 'action.close',
    search: 'action.search',
    'chevron-down': 'action.chevron-down',
    'chevron-right': 'action.chevron-right',
    'chevron-up': 'action.chevron-up',
    'chevron-left': 'action.chevron-left',
    minus: 'action.minus',
    more: 'action.more',
    filter: 'action.filter',
    calendar: 'action.calendar',
    upload: 'action.upload',
    download: 'action.download',
    sort: 'action.sort',
    external: 'action.external',
    'arrow-right': 'action.arrow-right',
    'arrow-left': 'action.arrow-left',
    trash: 'action.trash',
    edit: 'action.edit',
    refresh: 'action.refresh',
    history: 'action.history',
    moon: 'action.moon',
    sun: 'action.sun',
    monitor: 'action.monitor',
    info: 'status.info',
    warning: 'status.warning',
    success: 'status.success',
    danger: 'status.danger',
    menu: 'action.menu',
    plus: 'action.plus',
    home: 'nav.home',
    language: 'nav.language',
    copy: 'action.copy',
    discount: 'tool.percentage',
};
export function resolveIconName(name) {
    const key = String(name || '').trim();
    if (!key)
        return '';
    if (LEGACY_ICON_ALIASES[key])
        return LEGACY_ICON_ALIASES[key];
    return key;
}
export function resolveIconDef(name) {
    const key = resolveIconName(name);
    if (!key)
        return null;
    const def = REGISTRY[key];
    if (!def) {
        throw new Error(`@vmz/ui-icons: unknown icon name "${name}" (resolved: "${key}")`);
    }
    return def;
}
/** @deprecated Prefer `resolveIconDef`. Returns primary path data for fill icons or first stroke path. */
export function resolveIconPath(name) {
    const def = resolveIconDef(name);
    if (!def)
        return '';
    if (def.kind === 'fill')
        return def.d;
    if (def.kind === 'text-badge')
        return '';
    if (def.kind === 'stroke')
        return def.paths[0] ?? '';
    const pathNode = def.nodes.find((node) => node.tag === 'path');
    return pathNode?.attrs.d ?? '';
}
