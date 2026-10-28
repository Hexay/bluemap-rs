# Publishing to Modrinth and Hangar

`.github/workflows/publish.yml` runs `tools/publish.py` when a GitHub release is published (release.yml makes drafts,
so that's when you click Publish), on `workflow_dispatch` (re-publish a tag) and as a `workflow_call`. Drafts are
skipped. It publishes:

| Target | What | Version | Channel / type |
|---|---|---|---|
| Modrinth | Paper universal jar (loaders paper, folia, purpur) and Fabric universal jar (fabric, needs Fabric API) | `0.3.0-paper`, `0.3.0-fabric` (upstream: `5.29-paper`) | `-rc` etc. tags alpha, other `0.x` beta, else release |
| Hangar | Paper, as an external link to the release's universal jar (Hangar caps uploads at 10 MB; ours is ~40 MB) | `0.3.0` | Alpha / Beta / Release, same rule |

Changelog: the tag's `## <version>` section of `CHANGELOG.md` plus a link to the GitHub release. Game versions:
`SHIMS` in `tools/publish.py` (upstream 5.29's lists); update them with `fabric.mod.json` or a new verified Paper.
A target whose secret or variable is missing is skipped with a notice; a version already there is skipped.

Check locally (no network): `gh release download v0.3.0 -R Hexay/bluemap-rs -p '*-universal.jar' -D <dir>`, then
`py -3 tools/publish.py --tag v0.3.0 --jars <dir> --dry-run`.

## One-time setup

### Modrinth

1. modrinth.com → Create a project: name `bluemap-rs`, URL `bluemap-rs`, visibility Public. It stays a draft until
   reviewed.
2. Settings:
   - Summary: `Drop-in replacement for BlueMap 5.29 with a native Rust core: same configs, maps, webapp and addons,
     for a fraction of the CPU and memory. Unofficial, not affiliated with BlueMap.`
   - Categories: `utility`; additional: `management`, `optimization`. Client side Unsupported, server side Required.
   - License: MIT. Links: source `https://github.com/Hexay/bluemap-rs`, issues
     `https://github.com/Hexay/bluemap-rs/issues`, wiki `https://github.com/Hexay/bluemap-rs/blob/master/MIGRATING.md`.
   - Description: README from the intro through "Install", without the badges, relative links made absolute.
   - Icon: none in the repo. Don't use BlueMap's logo.
3. Token: Settings → Personal access tokens → Create, scopes **Create versions**, **Read versions**, **Read
   projects** (the read scopes let the duplicate check see a draft project's versions).
4. Project ID (the API wants the ID, not the slug): `curl -s -H "Authorization: <token>"
   https://api.modrinth.com/v2/project/bluemap-rs | jq -r .id`

### Hangar

1. hangar.papermc.io → New project: owner `Hexay`, platform Paper, name `bluemap-rs`, category Misc (as upstream),
   license MIT, description (max 120 chars): `Drop-in BlueMap 5.29 replacement with a native Rust core: same maps and
   addons, far less CPU and memory.` Keywords: `map`, `bluemap`, `web`, `3d`, `dynmap`; tag Supports Folia; link
   GitHub. Page: same README excerpt as Modrinth.
2. Channels: `Release` exists; add `Beta` and `Alpha`, both flagged Unstable.
3. API key: avatar menu → API Keys, permissions **create_version** and **view_public_info** only.

### GitHub

```sh
gh secret set MODRINTH_TOKEN -R Hexay/bluemap-rs          # paste the PAT
gh variable set MODRINTH_PROJECT_ID -R Hexay/bluemap-rs -b <project id>
gh secret set HANGAR_TOKEN -R Hexay/bluemap-rs            # paste the API key
gh variable set HANGAR_PROJECT -R Hexay/bluemap-rs -b bluemap-rs
```

### First publish

With `publish.yml` on `master`: `gh workflow run publish.yml -R Hexay/bluemap-rs -f tag=v0.3.0`, then
`gh run watch -R Hexay/bluemap-rs`. Then submit the Modrinth project for review (it needs a version first).
