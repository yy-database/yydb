/** Fixed native binding filenames staged under each `@yydb/yydb-<platform>` package `lib/`. */
export const NATIVE_ARTIFACTS = [
    {
        pkg: 'yydb-win32-x64',
        fileName: 'yydb-win32-x64-msvc.node',
        hostPlatform: 'win32',
        hostArch: 'x64',
        cargoRelease: 'yydb_napi.dll',
    },
    {
        pkg: 'yydb-linux-x64',
        fileName: 'yydb-linux-x64-gnu.node',
        hostPlatform: 'linux',
        hostArch: 'x64',
        cargoRelease: 'libyydb_napi.so',
    },
    {
        pkg: 'yydb-darwin-x64',
        fileName: 'yydb-darwin-x64.node',
        hostPlatform: 'darwin',
        hostArch: 'x64',
        cargoRelease: 'libyydb_napi.dylib',
    },
    {
        pkg: 'yydb-darwin-arm64',
        fileName: 'yydb-darwin-arm64.node',
        hostPlatform: 'darwin',
        hostArch: 'arm64',
        cargoRelease: 'libyydb_napi.dylib',
    },
];
