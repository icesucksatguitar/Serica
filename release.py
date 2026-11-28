#!/usr/bin/env python3
"""
Serica - one release for Windows and macOS
==========================================

Windows (./rust) and macOS (./macos) always ship the same version. This
script moves both to the next version, commits, tags and pushes; the tag
starts .github/workflows/release.yml, which builds the Windows installer
and the Mac disk image side by side and puts both on one GitHub release,
with one latest.json that installed copies on either system update from.

    python3 release.py patch          0.5.3 -> 0.5.4, commit, tag v0.5.4, push
    python3 release.py minor          0.5.3 -> 0.6.0
    python3 release.py 0.6.0          an exact version
    python3 release.py --check        both apps on the same version? (CI runs this)
    python3 release.py patch --no-push    commit and tag only

The workflow makes the release a draft: review it on GitHub and press
Publish (installed copies only see published releases). To publish straight
away, run the workflow by hand with "live" ticked (Actions > Release).
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
APPS = {"Windows": ROOT / "rust", "macOS": ROOT / "macos"}
CARGO_VERSION = re.compile(r'(^\[package\][\s\S]*?^version\s*=\s*")([^"]+)(")', re.MULTILINE)


def versions(app: Path) -> dict[str, str]:
    """Every place an app keeps its version."""
    out = {}
    for name in ("package.json", "package-lock.json"):
        p = app / name
        if p.exists():
            data = json.loads(p.read_text(encoding="utf-8"))
            out[name] = data.get("version", "?")
            if name == "package-lock.json" and "" in data.get("packages", {}):
                out[name + " (root package)"] = data["packages"][""].get("version", "?")
    m = CARGO_VERSION.search((app / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8"))
    out["src-tauri/Cargo.toml"] = m.group(2) if m else "?"
    out["src-tauri/tauri.conf.json"] = json.loads((app / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))["version"]
    return out


def set_version(app: Path, ver: str) -> None:
    for name in ("package.json", "package-lock.json"):
        p = app / name
        if p.exists():
            data = json.loads(p.read_text(encoding="utf-8"))
            data["version"] = ver
            if name == "package-lock.json" and "" in data.get("packages", {}):
                data["packages"][""]["version"] = ver
            p.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    cargo = app / "src-tauri" / "Cargo.toml"
    cargo.write_text(CARGO_VERSION.sub(rf"\g<1>{ver}\g<3>", cargo.read_text(encoding="utf-8"), count=1), encoding="utf-8")
    # Only the top-level "version" line, so the file keeps its layout.
    conf = app / "src-tauri" / "tauri.conf.json"
    text = conf.read_text(encoding="utf-8")
    conf.write_text(re.sub(r'^(  "version":\s*")[^"]+(")', rf"\g<1>{ver}\g<2>", text, count=1, flags=re.MULTILINE), encoding="utf-8")


def current() -> str | None:
    """The shared version, or None (and a report) when anything disagrees."""
    seen: dict[str, list[str]] = {}
    for label, app in APPS.items():
        for where, v in versions(app).items():
            seen.setdefault(v, []).append(f"{label}: {app.name}/{where}")
    if len(seen) == 1:
        return next(iter(seen))
    print("Windows and macOS are not on the same version:", file=sys.stderr)
    for v, places in sorted(seen.items()):
        print(f"  {v}", file=sys.stderr)
        for p in places:
            print(f"      {p}", file=sys.stderr)
    return None


def bump(ver: str, kind: str) -> str:
    if re.fullmatch(r"\d+\.\d+\.\d+", kind):
        return kind
    major, minor, patch = (int(x) for x in ver.split("."))
    return {
        "major": f"{major + 1}.0.0",
        "minor": f"{major}.{minor + 1}.0",
        "patch": f"{major}.{minor}.{patch + 1}",
    }[kind]


def git(*args: str, check: bool = True) -> str:
    r = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    if check and r.returncode != 0:
        sys.exit(f"git {' '.join(args)} failed:\n{r.stderr.strip()}")
    return r.stdout.strip()


def main() -> int:
    ap = argparse.ArgumentParser(description="One version, one release, for Windows and macOS.")
    ap.add_argument("bump", nargs="?", help="patch | minor | major | X.Y.Z")
    ap.add_argument("--check", action="store_true", help="Only check that both apps are on the same version")
    ap.add_argument("--tag", help="With --check: also require this tag (vX.Y.Z) to match")
    ap.add_argument("--no-push", action="store_true", help="Commit and tag, but don't push")
    args = ap.parse_args()

    ver = current()
    if args.check:
        if ver is None:
            return 1
        if args.tag and args.tag.lstrip("v") != ver:
            print(f"Tag {args.tag} doesn't match the apps' version {ver}. Tag v{ver}, or run release.py.", file=sys.stderr)
            return 1
        print(ver)
        return 0
    if not args.bump:
        ap.print_help()
        return 1
    if ver is None:
        return 1
    if args.bump not in ("patch", "minor", "major") and not re.fullmatch(r"\d+\.\d+\.\d+", args.bump):
        sys.exit(f"Not a version or bump: {args.bump}")
    if git("status", "--porcelain", "--untracked-files=no"):
        sys.exit("Uncommitted changes: commit or stash them first.")

    new = bump(ver, args.bump)
    tag = f"v{new}"
    if git("tag", "--list", tag):
        sys.exit(f"Tag {tag} already exists.")
    for app in APPS.values():
        set_version(app, new)
    assert current() == new
    git("add", "--", *(str(a.relative_to(ROOT)) for a in APPS.values()))
    git("commit", "-m", f"Version {new}")
    git("tag", "-a", tag, "-m", f"Serica {new}")
    print(f"{ver} -> {new}: committed and tagged {tag}.")
    if args.no_push:
        print(f"Push when ready: git push --atomic origin HEAD {tag}")
        return 0
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    git("push", "--atomic", "origin", f"HEAD:{branch}", tag)
    print(f"Pushed. The Release workflow is building Windows and macOS for {tag}:")
    remote = git("remote", "get-url", "origin").removesuffix(".git").replace("git@github.com:", "https://github.com/")
    print(f"  {remote}/actions/workflows/release.yml")
    return 0


if __name__ == "__main__":
    sys.exit(main())
