# Maintenance

| Path | Purpose |
|------|---------|
| `author-github.json` | Nifty commit author → GitHub login map |
| `releases/vX.Y.Z.md` | **Published** GitHub Release bodies (committed; edit here, then sync to GitHub) |

Local draft index only (gitignored): `releases/vX.Y.Z.reference.md` via `nifty change-logs --version X.Y.Z --write`.

Release bodies are English. Keep one file per git tag; `vX.Y.Z.md` is the source of truth for the release of that tag.
