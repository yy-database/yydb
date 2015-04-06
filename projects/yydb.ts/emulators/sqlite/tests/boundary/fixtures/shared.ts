export const READ_LIMITS = { maxPages: 64, maxRows: 32 } as const;

export interface TempFixture {
    directory: string;
    path: string;
}
