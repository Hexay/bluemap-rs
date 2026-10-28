"""Publishes a GitHub release's plugin and mod jars to Modrinth and Hangar (docs/release-publishing.md).

    py -3 tools/publish.py --tag v0.3.0 --jars DIR [--changelog CHANGELOG.md] [--only modrinth|hangar] [--dry-run]

DIR holds the release's `bluemap-rs-{paper,fabric}-<version>-universal.jar`. A target is skipped with a notice when
its token or project is unset (MODRINTH_TOKEN + MODRINTH_PROJECT_ID, HANGAR_TOKEN + HANGAR_PROJECT) and per version
when that version already exists there. `--dry-run` prints every request instead of sending it, with placeholders
for unset values; it never touches the network.
"""

import argparse
import hashlib
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO = os.environ.get("GITHUB_REPOSITORY", "Hexay/bluemap-rs")
MODRINTH_API = "https://api.modrinth.com/v2"
HANGAR_API = "https://hangar.papermc.io/api/v1"
USER_AGENT = f"{REPO}/publish (https://github.com/{REPO})"
FABRIC_API_PROJECT = "P7dR8mSH"
HANGAR_CHANNELS = {"release": "Release", "beta": "Beta", "alpha": "Alpha"}


@dataclass(frozen=True)
class Shim:
    name: str
    title: str
    loaders: tuple[str, ...]
    game_versions: tuple[str, ...]
    dependencies: tuple[dict, ...] = ()


# game versions as upstream 5.29 lists them (its implementations/{paper,fabric}/build.gradle.kts)
PAPER = Shim("paper", "Paper", ("paper", "folia", "purpur"), ("26.1.1", "26.1.2", "26.2", "26.3"))
FABRIC = Shim("fabric", "Fabric", ("fabric",), ("26.1", "26.1.1", "26.1.2", "26.2", "26.3"),
              ({"project_id": FABRIC_API_PROJECT, "version_id": None, "file_name": None,
                "dependency_type": "required"},))
SHIMS = (PAPER, FABRIC)


@dataclass(frozen=True)
class Release:
    tag: str
    version: str
    channel: str
    changelog: str
    jars: dict[str, Path]

    def asset_url(self, jar: Path) -> str:
        return f"https://github.com/{REPO}/releases/download/{self.tag}/{jar.name}"


def channel(tag: str) -> str:
    """`-suffix` tags (release.yml's PRERELEASE) are alpha; other 0.x tags beta, as mod sites label pre-1.0 versions."""
    if "-" in tag:
        return "alpha"
    return "beta" if tag.startswith("v0.") else "release"


def changelog_section(path: Path, version: str, tag: str) -> str:
    text = path.read_text(encoding="utf-8")
    m = re.search(rf"^## {re.escape(version)}(?=\s|$)[^\n]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not m:
        sys.exit(f"error: no `## {version}` section in {path}")
    return (f"{m.group(1).strip()}\n\nOther platforms (per-platform jars, CLI, Docker): "
            f"https://github.com/{REPO}/releases/tag/{tag}")


def load_release(tag: str, jars_dir: Path, changelog: Path) -> Release:
    version = tag.removeprefix("v")
    jars = {s.name: jars_dir / f"bluemap-rs-{s.name}-{version}-universal.jar" for s in SHIMS}
    if missing := [str(p) for p in jars.values() if not p.is_file()]:
        sys.exit(f"error: missing jars: {', '.join(missing)}")
    return Release(tag, version, channel(tag), changelog_section(changelog, version, tag), jars)


def notice(msg: str) -> None:
    print(f"::notice::{msg}" if os.environ.get("GITHUB_ACTIONS") else f"notice: {msg}", flush=True)


class HttpError(Exception):
    pass


class Http:
    """urllib with JSON responses; in dry-run mode it prints each request and returns (None, None)."""

    def __init__(self, dry_run: bool, secrets: list[str]):
        self.dry_run = dry_run
        self.secrets = [s for s in secrets if s]

    def redact(self, text: str) -> str:
        for s in self.secrets:
            text = text.replace(s, "<redacted>")
        return text

    def request(self, method: str, url: str, headers: dict[str, str] | None = None,
                body: "Multipart | None" = None, allow: tuple[int, ...] = ()) -> tuple[int | None, object]:
        headers = {"User-Agent": USER_AGENT, **(headers or {})}
        if body:
            headers["Content-Type"] = body.content_type
        if self.dry_run:
            print(self.redact(f"{method} {url}"))
            for k, v in headers.items():
                print(self.redact(f"  {k}: {v}"))
            if body:
                print(body.describe())
            return None, None
        req = urllib.request.Request(url, data=body.encode() if body else None, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                status, raw = r.status, r.read()
        except urllib.error.HTTPError as e:
            status, raw = e.code, e.read()
            if status not in allow:
                raise HttpError(self.redact(f"{method} {url}: HTTP {status}: {raw[:2000].decode(errors='replace')}"))
        return status, json.loads(raw) if raw.strip() else None


class Multipart:
    def __init__(self):
        self.boundary = uuid.uuid4().hex
        self.content_type = f"multipart/form-data; boundary={self.boundary}"
        self.parts: list[tuple[str, str | None, str, bytes]] = []

    def json(self, name: str, value: dict) -> "Multipart":
        self.parts.append((name, None, "application/json", json.dumps(value).encode()))
        return self

    def file(self, name: str, path: Path) -> "Multipart":
        self.parts.append((name, path.name, "application/java-archive", path.read_bytes()))
        return self

    def encode(self) -> bytes:
        out = bytearray()
        for name, filename, ctype, data in self.parts:
            disposition = f'form-data; name="{name}"' + (f'; filename="{filename}"' if filename else "")
            out += (f"--{self.boundary}\r\nContent-Disposition: {disposition}\r\n"
                    f"Content-Type: {ctype}\r\n\r\n").encode()
            out += data + b"\r\n"
        return bytes(out + f"--{self.boundary}--\r\n".encode())

    def describe(self) -> str:
        lines = []
        for name, filename, ctype, data in self.parts:
            if filename:
                lines.append(f"  part {name!r}: file {filename} ({ctype}, {len(data)} bytes, "
                             f"sha1 {hashlib.sha1(data).hexdigest()})")
            else:
                lines.append(f"  part {name!r} ({ctype}):")
                lines += [f"    {line}" for line in json.dumps(json.loads(data), indent=2).splitlines()]
        return "\n".join(lines)


def publish_modrinth(http: Http, rel: Release, token: str, project: str) -> None:
    auth = {"Authorization": token}
    _, versions = http.request("GET", f"{MODRINTH_API}/project/{project}/version", auth)
    existing = {v["version_number"] for v in versions or []}
    for shim in SHIMS:
        number = f"{rel.version}-{shim.name}"
        if number in existing:
            notice(f"Modrinth: {number} already exists, skipped")
            continue
        data = {
            "project_id": project,
            "name": f"{rel.version} ({shim.title})",
            "version_number": number,
            "changelog": rel.changelog,
            "version_type": rel.channel,
            "loaders": list(shim.loaders),
            "game_versions": list(shim.game_versions),
            "dependencies": list(shim.dependencies),
            "environment": "dedicated_server_only",
            "featured": False,
            "file_parts": ["jar"],
            "primary_file": "jar",
        }
        _, created = http.request("POST", f"{MODRINTH_API}/version", auth,
                                  Multipart().json("data", data).file("jar", rel.jars[shim.name]))
        print(f"Modrinth: published {number}" + (f" ({created['id']})" if created else " (dry run)"))


def publish_hangar(http: Http, rel: Release, api_key: str, project: str) -> None:
    slug = project.rsplit("/", 1)[-1]
    _, session = http.request("POST", f"{HANGAR_API}/authenticate?apiKey={urllib.parse.quote(api_key)}")
    # the JWT goes in bare, as hangar-publish-plugin sends it
    auth = {"Authorization": session["token"] if session else "<jwt from /authenticate>"}
    status, _ = http.request("GET", f"{HANGAR_API}/projects/{slug}/versions/{urllib.parse.quote(rel.version)}",
                             auth, allow=(404,))
    if status == 200:
        notice(f"Hangar: {rel.version} already exists, skipped")
        return
    # Hangar caps uploads at 10 MB; the universal jar (~40 MB) is linked from the GitHub release instead
    upload = {
        "version": rel.version,
        "channel": HANGAR_CHANNELS[rel.channel],
        "description": rel.changelog,
        "files": [{"platforms": ["PAPER"], "externalUrl": rel.asset_url(rel.jars[PAPER.name])}],
        "platformDependencies": {"PAPER": list(PAPER.game_versions)},
        "pluginDependencies": {},
    }
    _, created = http.request("POST", f"{HANGAR_API}/projects/{slug}/upload", auth,
                              Multipart().json("versionUpload", upload))
    print(f"Hangar: published {rel.version}" + (f" ({created['url']})" if created else " (dry run)"))


def target_config(dry_run: bool, token_env: str, project_env: str) -> tuple[str, str] | None:
    token, project = os.environ.get(token_env, ""), os.environ.get(project_env, "")
    if dry_run:
        return token or f"<{token_env}>", project or f"<{project_env}>"
    if not (token and project):
        notice(f"{token_env} or {project_env} not set, skipped")
        return None
    return token, project


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--tag", required=True)
    ap.add_argument("--jars", type=Path, required=True)
    ap.add_argument("--changelog", type=Path, default=ROOT / "CHANGELOG.md")
    ap.add_argument("--only", choices=("modrinth", "hangar"), action="append")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    rel = load_release(args.tag, args.jars, args.changelog)
    print(f"{rel.tag}: version {rel.version}, channel {rel.channel}")
    targets = {"modrinth": (publish_modrinth, "MODRINTH_TOKEN", "MODRINTH_PROJECT_ID"),
               "hangar": (publish_hangar, "HANGAR_TOKEN", "HANGAR_PROJECT")}
    http = Http(args.dry_run, [os.environ.get(t) for _, t, _ in targets.values()])
    failed = []
    for name, (publish, token_env, project_env) in targets.items():
        if args.only and name not in args.only:
            continue
        if not (config := target_config(args.dry_run, token_env, project_env)):
            continue
        print(f"--- {name}")
        try:
            publish(http, rel, *config)
        except (HttpError, OSError) as e:
            print(f"::error::{name}: {e}" if os.environ.get("GITHUB_ACTIONS") else f"error: {name}: {e}")
            failed.append(name)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
