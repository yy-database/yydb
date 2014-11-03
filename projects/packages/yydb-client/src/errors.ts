/**
 * @yydb/yydb-client consumes the same error surface as @yydb/yydb.
 *
 * YydbError / YydbErrorCodes / YydbClientErrorCodes / isYydbError are re-exported
 * from {@linkcode @yydb/yydb/shared/errors} so engine code, retryability and
 * params handling live in exactly one place. Client authors minting
 * client-local errors (connect failure, request-id mismatch, …) still use
 * {@link YydbClientErrorCodes} from this package via YydbErrorCodes or the
 * named export.
 */
export {
    YydbError,
    YydbErrorCodes,
    YydbClientErrorCodes,
    isYydbError,
} from '@yydb/yydb';
export type { YydbErrorCode, YydbErrorOptions } from '@yydb/yydb';
