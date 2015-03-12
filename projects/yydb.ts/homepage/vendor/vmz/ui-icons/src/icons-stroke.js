/** Outlined icon set — 24 grid, 1.75 stroke, 3–21 safe area. */
export const STROKE_ICONS = {
    'action.search': {
        kind: 'stroke',
        circles: [{ cx: 10.5, cy: 10.5, r: 7.5 }],
        paths: ['M15.8 15.8L20 20'],
    },
    'action.close': {
        kind: 'stroke',
        paths: ['M7 7L17 17', 'M17 7L7 17'],
    },
    'action.check': {
        kind: 'stroke',
        paths: ['M5 12L10 17L19 7'],
    },
    'action.chevron-down': {
        kind: 'stroke',
        paths: ['M7 10L12 15L17 10'],
    },
    'action.chevron-right': {
        kind: 'stroke',
        paths: ['M10 7L15 12L10 17'],
    },
    'action.menu': {
        kind: 'stroke',
        paths: ['M4 7H20', 'M4 12H20', 'M4 17H20'],
    },
    'action.plus': {
        kind: 'stroke',
        paths: ['M12 5V19', 'M5 12H19'],
    },
    'action.copy': {
        kind: 'stroke',
        paths: ['M9 9H17V17H9Z', 'M6 6H14V14'],
    },
    'action.chevron-up': {
        kind: 'stroke',
        paths: ['M7 14L12 9L17 14'],
    },
    'action.chevron-left': {
        kind: 'stroke',
        paths: ['M14 7L9 12L14 17'],
    },
    'action.minus': {
        kind: 'stroke',
        paths: ['M5 12H19'],
    },
    'action.more': {
        kind: 'stroke',
        paths: ['M6 12H6.01', 'M12 12H12.01', 'M18 12H18.01'],
    },
    'action.filter': {
        kind: 'stroke',
        paths: ['M4 6H20L14 13V19L10 17V13L4 6Z'],
    },
    'action.calendar': {
        kind: 'stroke',
        paths: ['M4 8H20V20H4Z', 'M4 12H20', 'M8 4V8', 'M16 4V8'],
    },
    'action.upload': {
        kind: 'stroke',
        paths: ['M12 16V4', 'M7 9L12 4L17 9', 'M4 16V20H20V16'],
    },
    'action.download': {
        kind: 'stroke',
        paths: ['M12 4V16', 'M7 11L12 16L17 11', 'M4 20H20'],
    },
    'action.sort': {
        kind: 'stroke',
        paths: ['M8 18V6', 'M5 9L8 6L11 9', 'M16 6V18', 'M13 15L16 18L19 15'],
    },
    'action.sort-asc': {
        kind: 'stroke',
        paths: ['M8 18V6', 'M5 9L8 6L11 9', 'M15 18H21'],
    },
    'action.sort-desc': {
        kind: 'stroke',
        paths: ['M8 6V18', 'M5 15L8 18L11 15', 'M15 6H21'],
    },
    'action.external': {
        kind: 'stroke',
        paths: ['M14 4H20V10', 'M20 4L11 13', 'M18 14V20H4V6H10'],
    },
    'action.arrow-right': {
        kind: 'stroke',
        paths: ['M4 12H20', 'M14 6L20 12L14 18'],
    },
    'action.arrow-left': {
        kind: 'stroke',
        paths: ['M20 12H4', 'M10 6L4 12L10 18'],
    },
    'action.trash': {
        kind: 'stroke',
        paths: ['M4 7H20', 'M9 7V4H15V7', 'M6 7L7 20H17L18 7', 'M10 11V16', 'M14 11V16'],
    },
    'action.edit': {
        kind: 'stroke',
        paths: ['M4 20H8L19 9L15 5L4 16V20Z'],
    },
    'action.refresh': {
        kind: 'stroke',
        paths: ['M20 12A8 8 0 1 1 17.5 6.5', 'M20 4V9H15'],
    },
    'action.history': {
        kind: 'stroke',
        circles: [{ cx: 12, cy: 12, r: 8.5 }],
        paths: ['M12 7V12L15.5 14'],
    },
    'action.moon': {
        kind: 'stroke',
        paths: ['M20 14.5A8.5 8.5 0 0 1 9.5 4 8.5 8.5 0 1 0 20 14.5Z'],
    },
    'action.sun': {
        kind: 'stroke',
        circles: [{ cx: 12, cy: 12, r: 4 }],
        paths: ['M12 2V4', 'M12 20V22', 'M2 12H4', 'M20 12H22', 'M5 5L6.5 6.5', 'M17.5 17.5L19 19', 'M19 5L17.5 6.5', 'M6.5 17.5L5 19'],
    },
    'action.monitor': {
        kind: 'stroke',
        paths: ['M4 5H20V17H4Z', 'M9 20H15', 'M12 17V20'],
    },
    'action.search-document': {
        kind: 'stroke',
        paths: ['M5 4H15L19 8V20H5Z', 'M15 4V8H19'],
        circles: [{ cx: 11, cy: 14, r: 2.5 }],
    },
    'nav.home': {
        kind: 'stroke',
        paths: ['M4 10.5L12 4L20 10.5', 'M6 10V19H18V10', 'M10 19V13H14V19'],
    },
    'nav.language': {
        kind: 'stroke',
        circles: [{ cx: 12, cy: 12, r: 9 }],
        paths: ['M12 3V21', 'M3 12H21', 'M5.5 7.5C8 5.5 16 5.5 18.5 7.5', 'M5.5 16.5C8 18.5 16 18.5 18.5 16.5'],
    },
    'status.info': {
        kind: 'stroke',
        circles: [{ cx: 12, cy: 12, r: 9 }],
        paths: ['M12 11V16'],
        dots: [{ cx: 12, cy: 8, r: 1 }],
    },
    'status.warning': {
        kind: 'stroke',
        paths: ['M12 4L20 19H4L12 4Z', 'M12 10V14'],
        dots: [{ cx: 12, cy: 17, r: 1 }],
    },
    'status.success': {
        kind: 'stroke',
        circles: [{ cx: 12, cy: 12, r: 9 }],
        paths: ['M8 12L11 15L16 9'],
    },
    'status.danger': {
        kind: 'stroke',
        circles: [{ cx: 12, cy: 12, r: 9 }],
        paths: ['M12 8V13'],
        dots: [{ cx: 12, cy: 16.5, r: 1 }],
    },
    'feature.incremental': {
        kind: 'stroke',
        circles: [
            { cx: 5, cy: 12, r: 2 },
            { cx: 19, cy: 12, r: 2 },
            { cx: 12, cy: 6, r: 2 },
        ],
        paths: ['M7 12H10', 'M14 12H17', 'M12 8V10', 'M8.2 11.2L11 8.5', 'M12.8 8.5L15.6 11.2'],
    },
    'feature.runtime': {
        kind: 'stroke',
        paths: ['M8.5 7.5H15.5V16.5H8.5Z', 'M11 10L14.5 12L11 14Z'],
    },
    'feature.deploy': {
        kind: 'stroke',
        paths: ['M4 16H20', 'M12 6V14', 'M8.5 10.5L12 6L15.5 10.5'],
    },
    'tool.url': {
        kind: 'stroke',
        paths: ['M10.5 8.5a3 3 0 0 1 4.2 0l.8.8a3 3 0 0 1 0 4.2l-.8.8', 'M13.5 15.5a3 3 0 0 1-4.2 0l-.8-.8a3 3 0 0 1 0-4.2l.8-.8'],
    },
    'tool.temperature': {
        kind: 'stroke',
        circles: [{ cx: 12, cy: 18, r: 3 }],
        paths: ['M12 4V15', 'M10 9H14'],
    },
    'tool.length': {
        kind: 'stroke',
        paths: ['M4 12H20', 'M6 9V15', 'M10 10V14', 'M14 10V14', 'M18 9V15', 'M4 12L7 9', 'M4 12L7 15', 'M20 12L17 9', 'M20 12L17 15'],
    },
    'tool.weight': {
        kind: 'stroke',
        paths: ['M6 8H18', 'M12 8V16', 'M8 16H16', 'M9 6H15'],
    },
    'tool.time-units': {
        kind: 'stroke',
        circles: [{ cx: 12, cy: 12, r: 9 }],
        paths: ['M12 7V12L15.5 14'],
    },
    'tool.color': {
        kind: 'stroke',
        paths: ['M12 4C8.5 4 6 6.5 6 10s2.5 6 6 6', 'M12 4c3.5 0 6 2.5 6 6s-2.5 6-6 6'],
        dots: [
            { cx: 8, cy: 10, r: 1.2 },
            { cx: 12, cy: 8, r: 1.2 },
            { cx: 16, cy: 10, r: 1.2 },
        ],
    },
    'tool.percentage': {
        kind: 'stroke',
        circles: [
            { cx: 9.5, cy: 9.5, r: 2 },
            { cx: 14.5, cy: 14.5, r: 2 },
        ],
        paths: ['M16 8L8 16'],
    },
    'tool.area': {
        kind: 'stroke',
        paths: ['M5 5H19V19H5Z', 'M8 8H16V16H8Z'],
    },
    'tool.volume': {
        kind: 'stroke',
        paths: ['M5 9H19V19H5Z', 'M8 6H16', 'M8 6V9', 'M16 6V9'],
    },
    'tool.data-size': {
        kind: 'stroke',
        paths: ['M5 7H19', 'M5 11H15', 'M5 15H19'],
    },
    'tool.ratio': {
        kind: 'stroke',
        paths: ['M5 7H19', 'M5 17H19', 'M8 10V14', 'M16 10V14'],
    },
    'tool.average': {
        kind: 'stroke',
        paths: ['M4 18H20', 'M6 15L10 9L14 13L18 7'],
    },
    'tool.tip': {
        kind: 'stroke',
        paths: ['M12 4C9.5 4 8 6 8 8.5c0 1.8 1 3.2 2.5 4V15h3v-2.5c1.5-.8 2.5-2.2 2.5-4C16 6 14.5 4 12 4Z', 'M9.5 18H14.5', 'M10 20H14'],
    },
    'tool.aspect-ratio': {
        kind: 'stroke',
        paths: ['M5 7H19V17H5Z', 'M8 9H16V15H8Z'],
    },
    'tool.bandwidth': {
        kind: 'stroke',
        paths: ['M4 12H6', 'M8 8V16', 'M11 10V14', 'M14 6V18', 'M17 9V15', 'M20 12H22'],
    },
    'tool.percent-change': {
        kind: 'stroke',
        paths: ['M5 17L12 9L15 13L19 6', 'M4 19H20'],
    },
    'tool.ohm': {
        kind: 'stroke',
        paths: ['M8 11c0-2.2 1.8-4 4-4s4 1.8 4 4', 'M8 11V16', 'M16 11V16', 'M8 16H16'],
    },
    'tool.speed': {
        kind: 'stroke',
        paths: ['M5.5 14A7 7 0 0 1 18.5 14', 'M12 12L15.5 8.5'],
    },
};
