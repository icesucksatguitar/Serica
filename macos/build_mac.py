#!/usr/bin/env python3
"""
==============================================================================
SERICA FOR MACOS - BUILD & RELEASE SCRIPT
==============================================================================
Every way to build, try out and ship Serica on the Mac (Tauri v2 + WebKit),
the counterpart of ../build.py for Windows. Run it with no arguments for a
menu. Everything it makes lands in macos/release/.

  TRY IT OUT (nothing is shipped, your real Serica profile is never touched)
    python3 build_mac.py --test              Build "Serica Test.app" and open it (own profile,
                                             own Keychain item, no auto-updates or UI patches)
    python3 build_mac.py --test --fresh      ...from a clean Cargo build and an empty test profile
    python3 build_mac.py --dev               Dev mode: hot reload while you edit the UI (test profile)
    python3 build_mac.py --test --automation ...with the verification hooks on 127.0.0.1:7599
                                             (src-tauri/src/automation.rs; never in a release)

  BUILD LOCALLY
    python3 build_mac.py                     Local app -> release/app/Serica.app
    python3 build_mac.py --run               ...and open it (uses your real profile)
    python3 build_mac.py --fresh             ...from a clean Cargo build

  SHIP TO USERS
    1. Full release. Users get an update prompt:
         python3 build_mac.py -b patch --bundle     .app + .dmg (ad-hoc signed, or with
                                                    $APPLE_SIGNING_IDENTITY), the signed update
                                                    archive and its latest.json entries
                                                    -> release/github
         git commit -am "macOS version X.Y.Z" && git push
         python3 build_mac.py --publish-release     upload to the GitHub release (a draft),
                                                    merging the Mac entries into its latest.json
         python3 build_mac.py --publish-release --live   ...or publish it straight away
    2. UI patch (React/CSS-only fixes), as on Windows but for the Mac build
       (ui-latest-macos.json, so the two never cross):
         python3 build_mac.py --ui-patch --notes "Fixed X"            stage in release/ui-patch
         python3 build_mac.py --ui-patch --notes "Fixed X" --publish  ...and upload it

  OTHER
    python3 build_mac.py --status            What's built and staged
    --universal                              Apple Silicon + Intel in one app (needs
                                             `rustup target add x86_64-apple-darwin`);
                                             otherwise the app is for this Mac's processor
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

cargo_bin_dir = Path.home() / ".cargo" / "bin"
if cargo_bin_dir.exists() and str(cargo_bin_dir) not in os.environ.get("PATH", ""):
    os.environ["PATH"] = f"{cargo_bin_dir}:{os.environ.get('PATH', '')}"

MACOS_DIR = Path(__file__).resolve().parent
ROOT_DIR = MACOS_DIR.parent
TAURI_DIR = MACOS_DIR / "src-tauri"
RELEASE_DIR = MACOS_DIR / "release"
APP_RELEASE_DIR = RELEASE_DIR / "app"
GITHUB_RELEASE_DIR = RELEASE_DIR / "github"
UI_PATCH_DIR = RELEASE_DIR / "ui-patch"
TEST_RELEASE_DIR = RELEASE_DIR / "test"
MANIFEST_FILE = RELEASE_DIR / "manifest.json"
TEST_CONFIG = TAURI_DIR / "tauri.test.conf.json"
TEST_TARGET_DIR = TAURI_DIR / "target-test"

GITHUB_REPO = "icesucksatguitar/Serica"
UPDATER_KEY = Path.home() / ".tauri" / "serica-updater.key"

CYAN = "\033[96m"
GREEN = "\033[92m"
YELLOW = "\033[93m"
RED = "\033[91m"
MAGENTA = "\033[95m"
BOLD = "\033[1m"
DIM = "\033[2m"
RESET = "\033[0m"


def log_header(title: str):
    print(f"\n{BOLD}{CYAN}=== {title} ==={RESET}")


def log_info(msg: str):
    print(f"{CYAN}{BOLD}[INFO]{RESET} {msg}")


def log_success(msg: str):
    print(f"{GREEN}{BOLD}[SUCCESS]{RESET} {msg}")


def log_warn(msg: str):
    print(f"{YELLOW}{BOLD}[WARNING]{RESET} {msg}")


def log_error(msg: str):
    print(f"{RED}{BOLD}[ERROR]{RESET} {msg}")


def run_cmd(cmd: list[str], cwd: Path, env: dict | None = None) -> bool:
    """Runs a command, streaming its output."""
    cmd_str = " ".join(str(c) for c in cmd)
    log_info(f"Running: {DIM}{cmd_str}{RESET} (in {cwd.relative_to(ROOT_DIR) if cwd.is_relative_to(ROOT_DIR) else cwd}/)")
    try:
        return subprocess.run(cmd, cwd=str(cwd), env=env or os.environ.copy(), check=True).returncode == 0
    except subprocess.CalledProcessError as e:
        log_error(f"Command failed with exit code {e.returncode}: {cmd_str}")
        return False
    except FileNotFoundError:
        log_error(f"Executable not found: {cmd[0]}")
        return False


def _git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=str(ROOT_DIR), capture_output=True, text=True)


def _cargo() -> str:
    return shutil.which("cargo") or str(cargo_bin_dir / "cargo")


def _copy_app(src: Path, dest: Path) -> bool:
    """Copies an .app bundle as it is (symlinks, signature, extended attributes)."""
    if dest.exists():
        shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    return run_cmd(["ditto", str(src), str(dest)], cwd=MACOS_DIR)


# ==============================================================================
# Versions (macos/ only: the Windows build keeps its own)
# ==============================================================================

def parse_semver(ver: str) -> tuple[int, int, int]:
    m = re.match(r"^(\d+)\.(\d+)\.(\d+)", ver.strip())
    if not m:
        raise ValueError(f"Invalid semver: '{ver}'")
    return int(m.group(1)), int(m.group(2)), int(m.group(3))


def bump_semver(ver: str, bump: str) -> str:
    if bump == "none":
        return ver.strip()
    major, minor, patch = parse_semver(ver)
    return {"major": f"{major + 1}.0.0", "minor": f"{major}.{minor + 1}.0", "patch": f"{major}.{minor}.{patch + 1}"}[bump]


def get_version() -> str:
    content = (TAURI_DIR / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"', content, re.MULTILINE)
    return m.group(1) if m else "0.0.0"


def set_version(new_ver: str):
    for name in ("package.json", "package-lock.json"):
        p = MACOS_DIR / name
        if p.exists():
            data = json.loads(p.read_text(encoding="utf-8"))
            data["version"] = new_ver
            if name == "package-lock.json" and "" in data.get("packages", {}):
                data["packages"][""]["version"] = new_ver
            p.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    cargo = TAURI_DIR / "Cargo.toml"
    cargo.write_text(
        re.sub(r'(^\[package\][\s\S]*?^version\s*=\s*")[^"]+(")', rf"\g<1>{new_ver}\g<2>", cargo.read_text(encoding="utf-8"), flags=re.MULTILINE),
        encoding="utf-8",
    )
    conf = TAURI_DIR / "tauri.conf.json"
    data = json.loads(conf.read_text(encoding="utf-8"))
    data["version"] = new_ver
    conf.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def load_manifest() -> dict:
    try:
        return json.loads(MANIFEST_FILE.read_text(encoding="utf-8"))
    except Exception:
        return {"project": "Serica for macOS", "last_updated": None, "releases": {}}


def save_manifest(manifest: dict):
    RELEASE_DIR.mkdir(parents=True, exist_ok=True)
    manifest["last_updated"] = datetime.now(timezone.utc).isoformat()
    MANIFEST_FILE.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def _dir_size(p: Path) -> int:
    return sum(f.stat().st_size for f in p.rglob("*") if f.is_file() and not f.is_symlink())


# ==============================================================================
# Targets: this Mac's processor, or Universal (Apple Silicon + Intel)
# ==============================================================================

def native_triple() -> str:
    return "aarch64-apple-darwin" if platform.machine() == "arm64" else "x86_64-apple-darwin"


def installed_targets() -> set[str]:
    out = subprocess.run(["rustup", "target", "list", "--installed"], capture_output=True, text=True)
    return set(out.stdout.split()) if out.returncode == 0 else {native_triple()}


def resolve_target(universal: bool) -> tuple[str | None, str]:
    """(--target for tauri, or None for the native one; the arch name Tauri
    puts in file names). Universal needs both Rust targets."""
    if universal:
        missing = {"aarch64-apple-darwin", "x86_64-apple-darwin"} - installed_targets()
        if missing:
            log_error(f"A Universal build needs these Rust targets: rustup target add {' '.join(sorted(missing))}")
            raise SystemExit(1)
        return "universal-apple-darwin", "universal"
    return None, "aarch64" if native_triple().startswith("aarch64") else "x64"


def bundle_dir(target_dir: Path, triple: str | None) -> Path:
    return target_dir / (triple or "") / "release" / "bundle" if triple else target_dir / "release" / "bundle"


def tauri_build(bundles: str, triple: str | None, env: dict, extra: list[str] | None = None) -> bool:
    cmd = ["npx", "tauri", "build", "--bundles", bundles]
    if triple:
        cmd += ["--target", triple]
    return run_cmd(cmd + (extra or []), cwd=MACOS_DIR, env=env)


def updater_env() -> tuple[dict, list[str], bool]:
    """Environment and config for tauri build: the update archive is signed
    with the updater key when it is there; without it no update archive is
    made (installed copies couldn't verify it)."""
    env = os.environ.copy()
    if UPDATER_KEY.exists():
        env["TAURI_SIGNING_PRIVATE_KEY"] = UPDATER_KEY.read_text(encoding="utf-8").strip()
        env.setdefault("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "")
        return env, [], True
    return env, ["--config", json.dumps({"bundle": {"createUpdaterArtifacts": False}})], False


def verify_signature(app: Path) -> bool:
    ok = subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], capture_output=True, text=True)
    if ok.returncode != 0:
        log_error(f"codesign could not verify {app.name}: {ok.stderr.strip()}")
        return False
    who = subprocess.run(["codesign", "-dv", str(app)], capture_output=True, text=True).stderr
    m = re.search(r"^Authority=(.+)$", who, re.MULTILINE)
    log_info(f"Signature: {m.group(1) if m else 'ad-hoc (runs on this Mac; sign with a Developer ID to distribute)'}")
    return True


# ==============================================================================
# Local build: release/app/Serica.app
# ==============================================================================

def build_app(bump: str, fresh: bool, universal: bool) -> bool:
    log_header("BUILDING SERICA FOR MACOS")
    current = get_version()
    new_ver = bump_semver(current, bump)
    if new_ver != current:
        log_info(f"Version: {BOLD}{current}{RESET} -> {GREEN}{BOLD}{new_ver}{RESET}")
        set_version(new_ver)
    else:
        log_info(f"Version: {BOLD}{current}{RESET} (no change)")
    if fresh:
        log_info("Clean Cargo build: removing previous build output...")
        if not run_cmd([_cargo(), "clean"], cwd=TAURI_DIR):
            return False
        shutil.rmtree(MACOS_DIR / "dist", ignore_errors=True)

    triple, arch = resolve_target(universal)
    env, extra, _ = updater_env()
    # tauri build runs `npm run build` (the UI) first, then cargo with the UI embedded.
    if not tauri_build("app", triple, env, extra):
        log_error("tauri build failed.")
        return False
    app = bundle_dir(TAURI_DIR / "target", triple) / "macos" / "Serica.app"
    if not app.exists():
        log_error(f"Expected app not found at {app}")
        return False
    if not verify_signature(app):
        return False
    dest = APP_RELEASE_DIR / "Serica.app"
    if not _copy_app(app, dest):
        return False
    size = _dir_size(dest)
    manifest = load_manifest()
    manifest["releases"]["app"] = {
        "version": new_ver,
        "path": str(dest.relative_to(MACOS_DIR)),
        "arch": arch,
        "size_mb": round(size / (1024 * 1024), 2),
        "built_at": datetime.now(timezone.utc).isoformat(),
    }
    save_manifest(manifest)
    log_success(f"App: {BOLD}{dest}{RESET} ({round(size / (1024 * 1024), 1)} MB, {arch})")
    return True


# ==============================================================================
# Full release: .app + .dmg + update archive, staged in release/github
# ==============================================================================

def build_bundle(universal: bool) -> bool:
    log_header("BUILDING THE APP, THE DISK IMAGE AND THE UPDATE ARCHIVE")
    version = get_version()
    triple, arch = resolve_target(universal)
    env, extra, signed_updates = updater_env()
    if not signed_updates:
        log_warn(f"Updater key not found at {UPDATER_KEY}: building the app and disk image without an update archive.")
        log_warn("Installed copies only take updates signed with that key.")
    if "APPLE_SIGNING_IDENTITY" in env:
        log_info(f"Code signing with: {env['APPLE_SIGNING_IDENTITY']}")
    if not tauri_build("app", triple, env, extra):
        log_error("tauri build failed.")
        return False
    # The disk image: its window is laid out by Finder (AppleScript). Without
    # a desktop session to drive Finder (locked screen, SSH), make it without
    # that layout rather than fail: the app and the Applications link are there.
    # (The app is bundled again with it: bundling the image alone removes the app.)
    out = bundle_dir(TAURI_DIR / "target", triple)
    bundle_cmd = ["npx", "tauri", "bundle", "--bundles", "app,dmg"] + (["--target", triple] if triple else []) + extra

    def clear_scratch():
        # Writable images a failed attempt leaves behind.
        for p in (out / "macos").glob("rw.*.dmg"):
            p.unlink()

    clear_scratch()
    if not run_cmd(bundle_cmd, cwd=MACOS_DIR, env=env):
        clear_scratch()
        log_warn("Finder couldn't lay out the disk image window (no desktop session?): making it without the custom layout.")
        plain = dict(env, CI="true")
        if not run_cmd(bundle_cmd, cwd=MACOS_DIR, env=plain):
            clear_scratch()
            log_error("Making the disk image failed.")
            return False

    app = out / "macos" / "Serica.app"
    dmg = out / "dmg" / f"Serica_{version}_{arch}.dmg"
    archive = out / "macos" / "Serica.app.tar.gz"
    sig = out / "macos" / "Serica.app.tar.gz.sig"
    needed = [app, dmg] + ([archive, sig] if signed_updates else [])
    for f in needed:
        if not f.exists():
            log_error(f"Expected build output not found: {f}")
            return False
    if not verify_signature(app):
        return False
    verify_dmg = subprocess.run(["hdiutil", "verify", str(dmg)], capture_output=True, text=True)
    if verify_dmg.returncode != 0:
        log_error(f"The disk image doesn't verify: {verify_dmg.stderr.strip()}")
        return False

    shutil.rmtree(GITHUB_RELEASE_DIR, ignore_errors=True)
    GITHUB_RELEASE_DIR.mkdir(parents=True)
    assets: dict[str, Path] = {dmg.name: dmg, f"Serica-macOS-{arch}.dmg": dmg}
    if signed_updates:
        archive_name = f"Serica_{version}_{arch}.app.tar.gz"
        assets[archive_name] = archive
        assets[archive_name + ".sig"] = sig
    for name, src in assets.items():
        shutil.copy2(src, GITHUB_RELEASE_DIR / name)

    if signed_updates:
        # The Mac entries of latest.json; --publish-release merges them into
        # the release's latest.json (which also lists Windows).
        entry = {
            "signature": sig.read_text(encoding="utf-8").strip(),
            "url": f"https://github.com/{GITHUB_REPO}/releases/download/v{version}/Serica_{version}_{arch}.app.tar.gz",
        }
        platforms = {"darwin-aarch64": entry, "darwin-x86_64": entry} if arch == "universal" else {f"darwin-{'aarch64' if arch == 'aarch64' else 'x86_64'}": entry}
        fragment = {
            "version": version,
            "notes": f"https://github.com/{GITHUB_REPO}/releases/tag/v{version}",
            "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "platforms": platforms,
        }
        (GITHUB_RELEASE_DIR / "latest-macos-fragment.json").write_text(json.dumps(fragment, indent=2) + "\n", encoding="utf-8")

    lines = []
    for p in sorted(GITHUB_RELEASE_DIR.iterdir()):
        if p.suffix in (".dmg", ".gz"):
            lines.append(f"{hashlib.sha256(p.read_bytes()).hexdigest()} *{p.name}")
    (GITHUB_RELEASE_DIR / "SHA256SUMS-macos.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")

    manifest = load_manifest()
    manifest["shipped"] = {"version": version, "commit": _git("rev-parse", "HEAD").stdout.strip()}
    manifest["releases"]["dmg"] = {
        "version": version,
        "path": str((GITHUB_RELEASE_DIR / dmg.name).relative_to(MACOS_DIR)),
        "arch": arch,
        "size_mb": round(dmg.stat().st_size / (1024 * 1024), 2),
        "built_at": datetime.now(timezone.utc).isoformat(),
    }
    if _git("status", "--porcelain", "--", "macos/src-tauri/src").stdout.strip():
        log_warn("Uncommitted Rust changes are in this build: commit them, or --ui-patch will think they're new.")
    save_manifest(manifest)
    _copy_app(app, APP_RELEASE_DIR / "Serica.app")

    log_success(f"Release assets for v{version} staged in {BOLD}{GITHUB_RELEASE_DIR}{RESET}")
    for p in sorted(GITHUB_RELEASE_DIR.iterdir()):
        print(f"   {p.name}")
    return True


def publish_release(live: bool) -> bool:
    """Uploads release/github to the GitHub release v<version> (shared with
    Windows), merging the Mac entries into the release's latest.json."""
    log_header("PUBLISHING THE MAC RELEASE TO GITHUB")
    version = get_version()
    tag = f"v{version}"
    if not any(GITHUB_RELEASE_DIR.glob("*.dmg")):
        log_error(f"Nothing staged in {GITHUB_RELEASE_DIR}. Run: python3 build_mac.py -b patch --bundle")
        return False
    fragment_path = GITHUB_RELEASE_DIR / "latest-macos-fragment.json"
    fragment = json.loads(fragment_path.read_text(encoding="utf-8")) if fragment_path.exists() else None
    if fragment and fragment.get("version") != version:
        log_error(f"Staged assets are v{fragment.get('version')}, but the code is v{version}. Rebuild with --bundle.")
        return False
    if _git("status", "--porcelain", "--untracked-files=no").stdout.strip():
        log_error("Uncommitted changes. Commit the version bump (and anything else in this build) first.")
        return False
    head = _git("rev-parse", "HEAD").stdout.strip()
    upstream = _git("rev-parse", "@{u}")
    if upstream.returncode != 0 or upstream.stdout.strip() != head:
        log_error("This commit isn't pushed (or the branch has no upstream). Push it first: git push")
        return False

    files = [p for p in sorted(GITHUB_RELEASE_DIR.iterdir()) if p.is_file() and p.name != fragment_path.name]
    exists = subprocess.run(["gh", "release", "view", tag, "--repo", GITHUB_REPO], capture_output=True, text=True).returncode == 0
    with tempfile.TemporaryDirectory() as tmp:
        if fragment:
            # latest.json lists every platform: keep what's there (Windows), add the Mac.
            merged = dict(fragment)
            if exists:
                got = subprocess.run(["gh", "release", "download", tag, "--repo", GITHUB_REPO, "-p", "latest.json", "-D", tmp], capture_output=True, text=True)
                current = Path(tmp) / "latest.json"
                if got.returncode == 0 and current.exists():
                    old = json.loads(current.read_text(encoding="utf-8"))
                    if old.get("version") == version:
                        merged = dict(old)
                        merged["platforms"] = {**old.get("platforms", {}), **fragment["platforms"]}
                    else:
                        log_warn(f"The release's latest.json is for v{old.get('version')}: replacing it with the Mac entries only.")
                    current.unlink()
            latest = Path(tmp) / "latest.json"
            latest.write_text(json.dumps(merged, indent=2) + "\n", encoding="utf-8")
            files.append(latest)
        names = [str(f) for f in files]
        if exists:
            log_info(f"Release {tag} exists: adding the Mac assets.")
            # latest.json last, so it never points at an archive still uploading.
            if not run_cmd(["gh", "release", "upload", tag, *names, "--clobber", "--repo", GITHUB_REPO], cwd=ROOT_DIR):
                return False
        else:
            if not run_cmd(["gh", "release", "create", tag, *names, "--repo", GITHUB_REPO, "--draft",
                            "--title", f"Serica {tag}", "--target", head, "--generate-notes"], cwd=ROOT_DIR):
                return False
    if live:
        if not run_cmd(["gh", "release", "edit", tag, "--repo", GITHUB_REPO, "--draft=false", "--latest"], cwd=ROOT_DIR):
            return False
        log_success(f"{tag} is live. Installed Mac copies offer the update on their next check.")
    else:
        log_success(f"{tag} uploaded (a draft until published). Review it on GitHub, or rerun with --live.")
    return True


# ==============================================================================
# UI patch (see src-tauri/src/ui_patch.rs): ui-latest-macos.json
# ==============================================================================

def rust_changed_since_release(version: str) -> list[str]:
    shipped = load_manifest().get("shipped", {})
    if shipped.get("version") == version and shipped.get("commit"):
        commit = shipped["commit"]
    else:
        commit = _git("log", "-1", "--format=%H", f"-Sversion = \"{version}\"", "--", "macos/src-tauri/Cargo.toml").stdout.strip()
    if not commit:
        return [f"(can't find the commit that set version {version} in macos/src-tauri/Cargo.toml)"]
    paths = ["macos/src-tauri/src", "macos/src-tauri/capabilities", "macos/src-tauri/Cargo.toml", "macos/src-tauri/tauri.conf.json", "macos/src-tauri/Info.plist"]
    changed = _git("diff", "--name-only", commit, "--", *paths).stdout.split()
    untracked = _git("ls-files", "--others", "--exclude-standard", "--", *paths).stdout.split()
    return [c for c in changed + untracked if not c.endswith(("Cargo.toml", "tauri.conf.json")) or
            any(l.startswith(("+", "-")) and "version" not in l and not l.startswith(("+++", "---"))
                for l in _git("diff", commit, "--", c).stdout.splitlines())]


def build_ui_patch(notes: str, publish: bool, force: bool) -> bool:
    import zipfile
    log_header("BUILDING A UI PATCH FOR THE MAC")
    version = get_version()
    if not UPDATER_KEY.exists():
        log_error(f"Signing key not found at {UPDATER_KEY}.")
        return False
    changed = rust_changed_since_release(version)
    if changed and not force:
        log_error(f"Rust code changed since v{version} was released. A UI patch can only carry")
        log_error("React/CSS changes; these need a normal release (python3 build_mac.py -b patch --bundle):")
        for c in changed:
            print(f"   {c}")
        log_info("If you're sure the UI doesn't depend on them, pass --force.")
        return False
    if not run_cmd(["npx", "tsc", "--noEmit", "-p", "."], cwd=MACOS_DIR):
        log_error("TypeScript errors: not shipping a broken UI.")
        return False
    if not run_cmd(["npm", "run", "build"], cwd=MACOS_DIR):
        return False

    build = int(datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S"))
    UI_PATCH_DIR.mkdir(parents=True, exist_ok=True)
    for old in UI_PATCH_DIR.iterdir():
        old.unlink()
    zip_name = f"ui-macos-{version}-{build}.zip"
    zip_path = UI_PATCH_DIR / zip_name
    dist = MACOS_DIR / "dist"
    meta = {"app_version": version, "build": build, "notes": notes, "platform": "macos"}
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
        for f in sorted(dist.rglob("*")):
            if f.is_file():
                z.write(f, f.relative_to(dist).as_posix())
        z.writestr("serica-ui.json", json.dumps(meta))

    env = os.environ.copy()
    env["TAURI_SIGNING_PRIVATE_KEY"] = UPDATER_KEY.read_text(encoding="utf-8").strip()
    env.setdefault("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "")
    if not run_cmd(["npx", "tauri", "signer", "sign", str(zip_path)], cwd=MACOS_DIR, env=env):
        log_error("Signing failed.")
        return False
    sig_path = zip_path.with_name(zip_name + ".sig")
    signature = sig_path.read_text(encoding="utf-8").strip()
    sig_path.unlink()

    tag = _git("describe", "--tags", "--abbrev=0").stdout.strip()
    gh = subprocess.run(["gh", "release", "view", "--repo", GITHUB_REPO, "--json", "tagName", "-q", ".tagName"], capture_output=True, text=True)
    if gh.returncode == 0 and gh.stdout.strip():
        tag = gh.stdout.strip()
    manifest = {
        "app_version": version,
        "build": build,
        "notes": notes,
        "url": f"https://github.com/{GITHUB_REPO}/releases/download/{tag}/{zip_name}",
        "signature": signature,
        "sha256": hashlib.sha256(zip_path.read_bytes()).hexdigest(),
    }
    manifest_path = UI_PATCH_DIR / "ui-latest-macos.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    log_success(f"UI patch {build} for v{version} ({round(zip_path.stat().st_size / 1024)} KB) staged in {UI_PATCH_DIR}")
    if not publish:
        log_info(f"Publish with: gh release upload {tag} \"{zip_path}\" \"{manifest_path}\" --clobber --repo {GITHUB_REPO}")
        log_info("(zip first, ui-latest-macos.json last) or rerun with --publish.")
        return True
    for f in (zip_path, manifest_path):
        if not run_cmd(["gh", "release", "upload", tag, str(f), "--clobber", "--repo", GITHUB_REPO], cwd=ROOT_DIR):
            return False
    assets = subprocess.run(["gh", "release", "view", tag, "--repo", GITHUB_REPO, "--json", "assets", "-q", ".assets[].name"],
                            capture_output=True, text=True).stdout.split()
    for name in assets:
        if name.startswith("ui-macos-") and name.endswith(".zip") and name != zip_name:
            subprocess.run(["gh", "release", "delete-asset", tag, name, "-y", "--repo", GITHUB_REPO])
    log_success(f"Published on {tag}. Running Mac copies of v{version} pick it up within 30 minutes (or on next launch).")
    return True


# ==============================================================================
# Test run and dev mode: an isolated copy you can try without shipping anything
# ==============================================================================

def _test_config() -> dict:
    return json.loads(TEST_CONFIG.read_text(encoding="utf-8"))


def _test_profile_dirs() -> list[Path]:
    ident = _test_config()["identifier"]
    lib = Path.home() / "Library"
    return [
        lib / "Application Support" / ident,
        lib / "Caches" / ident,
        lib / "WebKit" / ident,
        lib / "HTTPStorages" / ident,
        lib / "Saved Application State" / f"{ident}.savedState",
        lib / "Preferences" / f"{ident}.plist",
    ]


def build_test(fresh: bool, launch: bool, automation: bool = False) -> bool:
    """Builds "Serica Test.app" from the current code and opens it.

    Kept apart from the real browser: its own identifier
    (tauri.test.conf.json), so its own profile folders, website data,
    Keychain item and single-instance lock (it opens while Serica runs); no
    auto updates and no UI patches (debug assertions are on, which both
    check), so it always shows exactly the code you built. Its log is
    written next to it. The version and release/app are left alone."""
    log_header("TEST RUN")
    product = _test_config().get("productName", "Serica Test")
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(TEST_TARGET_DIR)  # keeps the real build's cache intact
    env["CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS"] = "true"
    env["CARGO_PROFILE_RELEASE_LTO"] = "false"
    env["CARGO_PROFILE_RELEASE_CODEGEN_UNITS"] = "16"
    if fresh:
        log_info("Clean Cargo build of the test copy...")
        shutil.rmtree(TEST_TARGET_DIR, ignore_errors=True)
    config = {"bundle": {"createUpdaterArtifacts": False}}
    log_info("Building the test copy (the first one takes a while; later ones are quick)...")
    cmd = ["npx", "tauri", "build", "--bundles", "app", "--config", str(TEST_CONFIG.relative_to(MACOS_DIR)), "--config", json.dumps(config)]
    if automation:
        cmd += ["--features", "automation"]
    if not run_cmd(cmd, cwd=MACOS_DIR, env=env):
        return False
    built = TEST_TARGET_DIR / "release" / "bundle" / "macos" / f"{product}.app"
    if not built.exists():
        log_error(f"Expected test app not found at {built}")
        return False
    running = subprocess.run(["pgrep", "-f", f"{product}.app/Contents/MacOS/"], capture_output=True, text=True)
    if running.stdout.strip():
        log_error(f"{product} is still running. Quit it (⌘Q) and run this again.")
        return False
    target = TEST_RELEASE_DIR / f"{product}.app"
    if not _copy_app(built, target):
        return False
    if fresh:
        for d in _test_profile_dirs():
            if d.exists():
                log_info(f"Clearing test profile: {d}")
                if d.is_dir():
                    shutil.rmtree(d, ignore_errors=True)
                else:
                    d.unlink()
        keychain = f"{product} Safe Storage"
        if subprocess.run(["security", "delete-generic-password", "-s", keychain], capture_output=True).returncode == 0:
            log_info(f"Removed the test copy's Keychain item ({keychain}).")
    log_success(f"Test copy: {BOLD}{target}{RESET}")
    log_info(f"Its profile: {_test_profile_dirs()[0]}")
    if launch:
        exe = next((target / "Contents" / "MacOS").iterdir())
        log_path = TEST_RELEASE_DIR / "serica-test.log"
        log = open(log_path, "w")
        run_env = os.environ.copy()
        if automation:
            run_env["SERICA_AUTOMATION_PORT"] = "7599"
        subprocess.Popen([str(exe)], cwd=str(TEST_RELEASE_DIR), stdout=log, stderr=subprocess.STDOUT, start_new_session=True, env=run_env)
        log_success(f"Opened {product}. Its log: {log_path}  (tail -f to follow)")
        if automation:
            log_info("Verification hooks: POST JSON to http://127.0.0.1:7599 (see src-tauri/src/automation.rs)")
    return True


def run_dev() -> bool:
    """`tauri dev` with hot reload, under the test identifier so it uses the
    test profile and runs alongside an installed Serica."""
    log_header("DEV MODE (hot reload, Control-C to stop)")
    return run_cmd(["npx", "tauri", "dev", "--config", str(TEST_CONFIG.relative_to(MACOS_DIR))], cwd=MACOS_DIR)


# ==============================================================================
# Status, menu, entry point
# ==============================================================================

def print_summary():
    log_header("STATUS")
    manifest = load_manifest()
    releases = manifest.get("releases", {})
    if not releases:
        log_warn("Nothing built yet.")
    for name, info in releases.items():
        path = MACOS_DIR / info.get("path", "")
        mark = f"{GREEN}✓ Ready{RESET}" if path.exists() else f"{RED}✗ Missing{RESET}"
        print(f"{BOLD}{name:<6}{RESET} v{info.get('version', '?'):<9} {info.get('arch', ''):<10} {info.get('size_mb', 0):>7} MB  {info.get('path')} [{mark}]")
    for p in sorted(TEST_RELEASE_DIR.glob("*.app")) if TEST_RELEASE_DIR.exists() else []:
        built = datetime.fromtimestamp(p.stat().st_mtime).strftime("%Y-%m-%d %H:%M")
        print(f"{BOLD}Test copy:{RESET}       {p} (built {built})")
    if any(GITHUB_RELEASE_DIR.glob("*.dmg")) if GITHUB_RELEASE_DIR.exists() else False:
        print(f"{BOLD}Staged release:{RESET}  {GITHUB_RELEASE_DIR} (upload: --publish-release)")
    ui = UI_PATCH_DIR / "ui-latest-macos.json"
    if ui.exists():
        m = json.loads(ui.read_text(encoding="utf-8"))
        print(f"{BOLD}Staged UI patch:{RESET} build {m.get('build')} for v{m.get('app_version')}: {m.get('notes') or '(no notes)'}")
    print(f"{BOLD}Version in the code:{RESET} {get_version()}   {BOLD}Rust targets:{RESET} {', '.join(sorted(installed_targets()))}")


def menu_args() -> list[str] | None:
    options = [
        ("Test run: build Serica Test.app and open it", ["--test"]),
        ("Test run from a clean build and an empty test profile", ["--test", "--fresh"]),
        ("Dev mode: hot reload while editing", ["--dev"]),
        ("Local build: release/app/Serica.app", []),
        ("Full release: bump the version, build the .app, .dmg and update archive", None),
        ("Publish the staged release to GitHub (as a draft)", ["--publish-release"]),
        ("UI patch: ship a React/CSS fix without a reinstall", None),
        ("Status: what's built and staged", ["--status"]),
    ]
    print(f"{BOLD}{MAGENTA}Serica for macOS build{RESET}  (python3 build_mac.py --help lists every flag)\n")
    for i, (label, _) in enumerate(options, 1):
        print(f"  {BOLD}{i}{RESET}  {label}")
    try:
        choice = input("\nChoose 1-8 (Enter to cancel): ").strip()
        if not choice:
            return None
        label, args = options[int(choice) - 1]
        if args is not None:
            return args
        if label.startswith("Full release"):
            bump = input("Bump: patch / minor / major [patch]: ").strip() or "patch"
            return ["--bump", bump, "--bundle"]
        notes = input("One line shown to users (e.g. \"Smoother sidebar\"): ").strip()
        publish = input("Upload it now so users get it? [y/N]: ").strip().lower() == "y"
        return ["--ui-patch", "--notes", notes] + (["--publish"] if publish else [])
    except (ValueError, IndexError):
        log_error("Not an option.")
        return None
    except (EOFError, KeyboardInterrupt):
        return None


def main() -> int:
    if sys.platform != "darwin":
        log_error("build_mac.py builds the macOS app and runs on a Mac. On Windows, use ../build.py.")
        return 1
    parser = argparse.ArgumentParser(
        description="Serica for macOS - build, try out and ship. Run with no arguments for a menu.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(__doc__ or "").split("menu. Everything it makes lands in macos/release/.\n", 1)[-1],
    )
    try_out = parser.add_argument_group("try it out (nothing is shipped)")
    try_out.add_argument("--test", action="store_true", help="Build Serica Test.app (own profile, no auto-updates) and open it")
    try_out.add_argument("--no-launch", action="store_true", help="With --test: build only, don't open it")
    try_out.add_argument("--dev", action="store_true", help="Dev mode: tauri dev with hot reload, on the test profile")
    try_out.add_argument("--automation", action="store_true", help="With --test: build in the verification hooks (127.0.0.1:7599)")

    build = parser.add_argument_group("build and full release")
    build.add_argument("-b", "--bump", choices=["major", "minor", "patch", "none"], default="none", help="Version bump. Default: none")
    build.add_argument("--fresh", action="store_true", help="Clean Cargo build first (with --test: also an empty test profile)")
    build.add_argument("--universal", action="store_true", help="Apple Silicon + Intel in one app")
    build.add_argument("--bundle", action="store_true", help="Build the .app, .dmg and signed update archive, staged in release/github")
    build.add_argument("--publish-release", action="store_true", help="Upload what --bundle staged to the GitHub release v<version>")
    build.add_argument("--live", action="store_true", help="With --publish-release: publish it and mark it Latest")
    build.add_argument("--run", action="store_true", help="Open the local build afterwards (uses your real profile)")

    patch = parser.add_argument_group("UI patch (no reinstall)")
    patch.add_argument("--ui-patch", action="store_true", help="Ship only the UI (React/CSS) as a signed patch for the current version")
    patch.add_argument("--notes", default="", help="With --ui-patch: one line shown to users")
    patch.add_argument("--publish", action="store_true", help="With --ui-patch: upload it to the GitHub release")
    patch.add_argument("--force", action="store_true", help="With --ui-patch: ship even though Rust code changed")

    parser.add_argument("--status", action="store_true", help="Show what's built and staged")

    argv = sys.argv[1:]
    if not argv and sys.stdin.isatty():
        argv = menu_args()
        if argv is None:
            return 0
    args = parser.parse_args(argv)

    if args.status:
        print_summary()
        return 0
    if args.test:
        return 0 if build_test(args.fresh, launch=not args.no_launch, automation=args.automation) else 1
    if args.dev:
        return 0 if run_dev() else 1
    if args.ui_patch:
        return 0 if build_ui_patch(args.notes, args.publish, args.force) else 1
    if args.publish_release:
        return 0 if publish_release(args.live) else 1

    print(f"{BOLD}{MAGENTA}================================================================{RESET}")
    print(f"{BOLD}{MAGENTA}              SERICA FOR MACOS - BUILD SYSTEM                  {RESET}")
    print(f"{BOLD}{MAGENTA}================================================================{RESET}")
    if args.bundle:
        current = get_version()
        new_ver = bump_semver(current, args.bump)
        if new_ver != current:
            log_info(f"Version: {BOLD}{current}{RESET} -> {GREEN}{BOLD}{new_ver}{RESET}")
            set_version(new_ver)
        if args.fresh and not run_cmd([_cargo(), "clean"], cwd=TAURI_DIR):
            return 1
        if not build_bundle(args.universal):
            log_error("Bundling failed.")
            return 1
        log_info("Next: commit and push, then upload the release:")
        log_info(f'  git commit -am "macOS version {get_version()}" && git push')
        log_info("  python3 build_mac.py --publish-release        (a draft; add --live to publish at once)")
    elif not build_app(args.bump, args.fresh, args.universal):
        log_error("Build failed.")
        return 1

    print_summary()
    log_success("Build completed successfully!")
    if args.run:
        app = APP_RELEASE_DIR / "Serica.app"
        log_info(f"Opening {app}")
        subprocess.Popen(["open", str(app)])
    return 0


if __name__ == "__main__":
    sys.exit(main())
