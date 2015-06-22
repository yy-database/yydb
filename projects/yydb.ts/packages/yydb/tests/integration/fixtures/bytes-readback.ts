// Reads every `Blob` row from one database file and prints the octets as hex.
//
// The parent test runs this in a separate Node process so the reopen case
// observes a real process exit rather than a handle that is still open.

import { Database } from "@yydb/yydb";

const path = process.argv[2];

const database = await Database.open(path);
const rows = await database.query("Blob.all().collect()");

const payloads = rows.map((row: { blob_id: number; body: Uint8Array }) => ({
    id: Number(row.blob_id),
    hex: Buffer.from(row.body).toString("hex"),
}));

process.stdout.write(JSON.stringify(payloads));
