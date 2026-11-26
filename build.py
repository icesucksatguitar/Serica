#!/usr/bin/env python3
"""
==============================================================================
SERICA BROWSER - BUILD & RELEASE SCRIPT
==============================================================================
Every way to build, try out and ship Serica. Run it with no arguments for a
menu.

  TRY IT OUT (nothing is shipped, your real Serica profile is never touched)
    python build.py --test              Test run: build an isolated test copy and open it
    python build.py --test --fresh      ...starting from an empty test profile
    python build.py --dev               Dev mode: hot reload while you edit the UI

  BUILD LOCALLY
    python build.py                     Local release exe -> release/rust/Serica.exe
    python build.py --run               ...and open it (uses your real profile)

  SHIP TO USERS
    1. Full release (anything changed, Rust included). Users get an update
       prompt and the installer runs:
         python build.py -b patch --bundle      signed MSI + latest.json -> release/github
         git commit -am "Version X.Y.Z" && git push
         python build.py --publish-release      upload as a draft GitHub release
         python build.py --publish-release --live   ...or publish it straight away
    2. UI patch (React/CSS-only fixes). No installer, no reinstall: running
       copies download it in the background (within 30 min, or 15 s after
       launch), offer a one-click "Reload" that swaps it in within a second,
       and roll it back by themselves if it fails to render. Refused when
       Rust code changed since the release users run:
         python build.py --ui-patch --notes "Fixed X"            stage in release/ui-patch
         python build.py --ui-patch --notes "Fixed X" --publish  ...and upload it

  OTHER
    python build.py --status            What's built and staged
"""

import os
import sys
import shutil
import subprocess
import argparse
import json
import re
from datetime import datetime, timezone
from pathlib import Path

# Enable ANSI escape sequences on Windows consoles
if os.name == "nt":
    os.system("")
# Piped or redirected output defaults to the ANSI code page on Windows, which
# can't encode the summary's ✓/✗ marks and crashed the end of every build.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass

cargo_bin_dir = Path.home() / ".cargo" / "bin"
if cargo_bin_dir.exists() and str(cargo_bin_dir) not in os.environ.get("PATH", ""):
    sep = ";" if os.name == "nt" else ":"
    os.environ["PATH"] = f"{cargo_bin_dir}{sep}{os.environ.get('PATH', '')}"

ROOT_DIR = Path(__file__).resolve().parent
RUST_DIR = ROOT_DIR / "rust"
RELEASE_DIR = ROOT_DIR / "release"
RUST_RELEASE_DIR = RELEASE_DIR / "rust"
MANIFEST_FILE = RELEASE_DIR / "manifest.json"

# ANSI Colors
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
    """Execute a system command, streaming output."""
    cmd_str = " ".join(str(c) for c in cmd)
    log_info(f"Running: {DIM}{cmd_str}{RESET} (in {cwd.name}/)")
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(cwd),
            env=env or os.environ.copy(),
            check=True
        )
        return proc.returncode == 0
    except subprocess.CalledProcessError as e:
        log_error(f"Command failed with exit code {e.returncode}: {cmd_str}")
        return False
    except FileNotFoundError:
        log_error(f"Executable not found for command: {cmd[0]}")
        return False


# ==============================================================================
# SemVer Management
# ==============================================================================

def parse_semver(ver_str: str) -> tuple[int, int, int]:
    m = re.match(r"^(\d+)\.(\d+)\.(\d+)", ver_str.strip())
    if not m:
        raise ValueError(f"Invalid semver: '{ver_str}'")
    return int(m.group(1)), int(m.group(2)), int(m.group(3))


def bump_semver(ver_str: str, bump_type: str) -> str:
    if bump_type == "none":
        return ver_str.strip()
    major, minor, patch = parse_semver(ver_str)
    if bump_type == "major":
        return f"{major + 1}.0.0"
    elif bump_type == "minor":
        return f"{major}.{minor + 1}.0"
    elif bump_type == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise ValueError(f"Unknown bump type: {bump_type}")


def get_rust_version() -> str:
    cargo_path = RUST_DIR / "src-tauri" / "Cargo.toml"
    if cargo_path.exists():
        content = cargo_path.read_text(encoding="utf-8")
        m = re.search(r'^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"', content, re.MULTILINE)
        if m:
            return m.group(1)
    pkg_path = RUST_DIR / "package.json"
    if pkg_path.exists():
        with open(pkg_path, "r", encoding="utf-8") as f:
            data = json.load(f)
        return data.get("version", "0.0.0")
    return "0.0.0"


def set_rust_version(new_ver: str):
    # 1. rust/package.json
    pkg_path = RUST_DIR / "package.json"
    if pkg_path.exists():
        with open(pkg_path, "r", encoding="utf-8") as f:
            data = json.load(f)
        data["version"] = new_ver
        with open(pkg_path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
            f.write("\n")

    # 2. rust/package-lock.json
    lock_path = RUST_DIR / "package-lock.json"
    if lock_path.exists():
        try:
            with open(lock_path, "r", encoding="utf-8") as f:
                data = json.load(f)
            data["version"] = new_ver
            if "packages" in data and "" in data["packages"]:
                data["packages"][""]["version"] = new_ver
            with open(lock_path, "w", encoding="utf-8") as f:
                json.dump(data, f, indent=2)
                f.write("\n")
        except Exception as e:
            log_warn(f"Failed to update rust/package-lock.json: {e}")

    # 3. rust/src-tauri/Cargo.toml
    cargo_path = RUST_DIR / "src-tauri" / "Cargo.toml"
    if cargo_path.exists():
        content = cargo_path.read_text(encoding="utf-8")
        updated = re.sub(
            r'(^\[package\][\s\S]*?^version\s*=\s*")[^"]+(")',
            rf'\g<1>{new_ver}\g<2>',
            content,
            flags=re.MULTILINE
        )
        cargo_path.write_text(updated, encoding="utf-8")

    # 4. rust/src-tauri/tauri.conf.json
    tauri_conf = RUST_DIR / "src-tauri" / "tauri.conf.json"
    if tauri_conf.exists():
        with open(tauri_conf, "r", encoding="utf-8") as f:
            data = json.load(f)
        data["version"] = new_ver
        with open(tauri_conf, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
            f.write("\n")


# ==============================================================================
# Manifest Management
# ==============================================================================

def load_manifest() -> dict:
    if MANIFEST_FILE.exists():
        try:
            with open(MANIFEST_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            pass
    return {
        "project": "Serica Browser",
        "last_updated": None,
        "releases": {}
    }


def update_manifest(env_name: str, version: str, exe_rel_path: str, size_bytes: int):
    RELEASE_DIR.mkdir(parents=True, exist_ok=True)
    manifest = load_manifest()
    manifest["last_updated"] = datetime.now(timezone.utc).isoformat()
    manifest["releases"][env_name] = {
        "version": version,
        "executable": exe_rel_path.replace("\\", "/"),
        "size_mb": round(size_bytes / (1024 * 1024), 2),
        "built_at": datetime.now(timezone.utc).isoformat(),
        "status": "ready"
    }
    with open(MANIFEST_FILE, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    log_info(f"Updated release manifest: {MANIFEST_FILE}")


# ==============================================================================
# Rust (Tauri v2) Build & Packaging
# ==============================================================================

def build_rust(bump_type: str, clean: bool = False) -> bool:
    log_header("BUILDING RUST (TAURI v2) ENVIRONMENT")
    npm_cmd = "npm.cmd" if os.name == "nt" else "npm"

    # Version Handling
    current_ver = get_rust_version()
    new_ver = bump_semver(current_ver, bump_type)
    if new_ver != current_ver:
        log_info(f"Incrementing Rust version: {BOLD}{current_ver}{RESET} -> {GREEN}{BOLD}{new_ver}{RESET}")
        set_rust_version(new_ver)
    else:
        log_info(f"Rust version: {BOLD}{current_ver}{RESET} (no change)")

    if clean:
        log_info("Cleaning previous Rust build artifacts...")
        shutil.rmtree(RUST_DIR / "dist", ignore_errors=True)
        shutil.rmtree(RUST_RELEASE_DIR, ignore_errors=True)

    # 1. Build Vite frontend
    log_info("Building Vite React UI for Tauri...")
    if not run_cmd([npm_cmd, "run", "build"], cwd=RUST_DIR):
        log_error("Rust Vite frontend build failed.")
        return False

    # 2. Build Rust native core via Cargo
    log_info("Building Rust native binary (cargo build --release --features custom-protocol)...")
    cargo_bin = shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / ("cargo.exe" if os.name == "nt" else "cargo"))
    # Without `custom-protocol` the exe loads the UI from the Vite dev server
    # (devUrl) and shows a blank window when it is not running.
    cargo_cmd = [cargo_bin, "build", "--release", "--features", "custom-protocol"]
    cargo_cwd = RUST_DIR / "src-tauri"
    if not run_cmd(cargo_cmd, cwd=cargo_cwd):
        log_error("Cargo release build failed.")
        return False

    # 3. Locate compiled binary
    exe_name = "serica-rust.exe" if os.name == "nt" else "serica-rust"
    built_bin = RUST_DIR / "src-tauri" / "target" / "release" / exe_name
    if not built_bin.exists():
        log_error(f"Expected compiled binary not found at {built_bin}")
        return False

    # 4. Stage into unified root release/rust
    RUST_RELEASE_DIR.mkdir(parents=True, exist_ok=True)
    target_exe = RUST_RELEASE_DIR / "Serica.exe"
    shutil.copy2(built_bin, target_exe)
    
    # Also copy as serica-rust.exe for explicit naming
    target_named = RUST_RELEASE_DIR / exe_name
    if target_named != target_exe:
        shutil.copy2(built_bin, target_named)

    # Copy icons / assets if available
    icons_src = RUST_DIR / "src-tauri" / "icons"
    if icons_src.exists():
        dest_icons = RUST_RELEASE_DIR / "icons"
        shutil.copytree(icons_src, dest_icons, dirs_exist_ok=True)

    size_bytes = target_exe.stat().st_size
    log_success(f"Rust release built successfully!")
    log_success(f"Executable: {BOLD}{target_exe}{RESET} ({round(size_bytes / (1024 * 1024), 2)} MB)")

    rel_exe = str(target_exe.relative_to(ROOT_DIR))
    update_manifest("rust", new_ver, rel_exe, size_bytes)
    return True


# ==============================================================================
# Signed installer + auto-update manifest (GitHub release assets)
# ==============================================================================

GITHUB_REPO = "icesucksatguitar/Serica"
UPDATER_KEY = Path.home() / ".tauri" / "serica-updater.key"
GITHUB_RELEASE_DIR = RELEASE_DIR / "github"


def build_bundle() -> bool:
    """Builds the signed MSI with `tauri build` and stages every asset a
    GitHub release needs into release/github, including latest.json, the
    manifest installed copies poll for updates
    (releases/latest/download/latest.json). Upload all of it to the release."""
    log_header("BUILDING SIGNED INSTALLER + UPDATE MANIFEST")
    if not UPDATER_KEY.exists():
        log_error(f"Updater signing key not found at {UPDATER_KEY}. Without it installed copies can't verify updates.")
        return False
    version = get_rust_version()
    env = os.environ.copy()
    env["TAURI_SIGNING_PRIVATE_KEY"] = UPDATER_KEY.read_text(encoding="utf-8").strip()
    env.setdefault("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "")
    npx_cmd = "npx.cmd" if os.name == "nt" else "npx"
    if not run_cmd([npx_cmd, "tauri", "build", "--bundles", "msi"], cwd=RUST_DIR, env=env):
        log_error("tauri build failed.")
        return False

    target = RUST_DIR / "src-tauri" / "target" / "release"
    msi = target / "bundle" / "msi" / f"Serica_{version}_x64_en-US.msi"
    sig = msi.with_name(msi.name + ".sig")
    exe = target / "serica-rust.exe"
    for f in (msi, sig, exe):
        if not f.exists():
            log_error(f"Expected build output not found: {f}")
            return False

    shutil.rmtree(GITHUB_RELEASE_DIR, ignore_errors=True)
    GITHUB_RELEASE_DIR.mkdir(parents=True)
    assets = {
        msi.name: msi,
        "Serica-Setup-x64.msi": msi,
        f"Serica_{version}_x64_portable.exe": exe,
        sig.name: sig,
    }
    for name, src in assets.items():
        shutil.copy2(src, GITHUB_RELEASE_DIR / name)

    latest = {
        "version": version,
        "notes": f"https://github.com/{GITHUB_REPO}/releases/tag/v{version}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {
            "windows-x86_64": {
                "signature": sig.read_text(encoding="utf-8").strip(),
                "url": f"https://github.com/{GITHUB_REPO}/releases/download/v{version}/{msi.name}",
            }
        },
    }
    (GITHUB_RELEASE_DIR / "latest.json").write_text(json.dumps(latest, indent=2) + "\n", encoding="utf-8")

    import hashlib
    lines = []
    for name in sorted(p.name for p in GITHUB_RELEASE_DIR.iterdir() if p.suffix in (".msi", ".exe")):
        digest = hashlib.sha256((GITHUB_RELEASE_DIR / name).read_bytes()).hexdigest()
        lines.append(f"{digest} *{name}")
    (GITHUB_RELEASE_DIR / "SHA256SUMS.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")

    # Lets --ui-patch check the UI still matches the Rust code in this build.
    manifest = load_manifest()
    manifest["shipped"] = {"version": version, "commit": _git("rev-parse", "HEAD").stdout.strip()}
    if _git("status", "--porcelain", "--", "rust/src-tauri/src").stdout.strip():
        log_warn("Uncommitted Rust changes are in this build: commit them, or --ui-patch will think they're new.")
    MANIFEST_FILE.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

    log_success(f"Release assets for v{version} staged in {BOLD}{GITHUB_RELEASE_DIR}{RESET}")
    for p in sorted(GITHUB_RELEASE_DIR.iterdir()):
        print(f"   {p.name}")
    return True


# ==============================================================================
# UI patch: ship a UI-only fix without an installer (see src-tauri/src/ui_patch.rs)
# ==============================================================================

UI_PATCH_DIR = RELEASE_DIR / "ui-patch"


def _git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=str(ROOT_DIR), capture_output=True, text=True)


def rust_changed_since_release(version: str) -> list[str]:
    """Rust files changed since the commit that set the current version, i.e.
    since the binary users run was built. A UI built on top of those changes
    could call commands the installed binary doesn't have."""
    shipped = load_manifest().get("shipped", {})
    if shipped.get("version") == version and shipped.get("commit"):
        commit = shipped["commit"]  # recorded by --bundle: exactly what users run
    else:
        found = _git("log", "-1", "--format=%H", f"-Sversion = \"{version}\"", "--", "rust/src-tauri/Cargo.toml")
        commit = found.stdout.strip()
    if not commit:
        return ["(can't find the commit that set version " + version + " in Cargo.toml)"]
    paths = ["rust/src-tauri/src", "rust/src-tauri/capabilities", "rust/src-tauri/Cargo.toml", "rust/src-tauri/tauri.conf.json"]
    changed = _git("diff", "--name-only", commit, "--", *paths).stdout.split()
    untracked = _git("ls-files", "--others", "--exclude-standard", "--", *paths).stdout.split()
    # The version bump itself touches Cargo.toml/tauri.conf.json; only real edits count.
    return [c for c in changed + untracked if not c.endswith(("Cargo.toml", "tauri.conf.json")) or
            any(l.startswith(("+", "-")) and "version" not in l and not l.startswith(("+++", "---"))
                for l in _git("diff", commit, "--", c).stdout.splitlines())]


def build_ui_patch(notes: str, publish: bool, force: bool) -> bool:
    log_header("BUILDING UI PATCH")
    import hashlib, zipfile
    version = get_rust_version()
    if not UPDATER_KEY.exists():
        log_error(f"Signing key not found at {UPDATER_KEY}.")
        return False

    changed = rust_changed_since_release(version)
    if changed and not force:
        log_error("Rust code changed since v" + version + " was released. A UI patch can only carry")
        log_error("React/CSS changes; these need a normal release (python build.py -b patch --bundle):")
        for c in changed:
            print(f"   {c}")
        log_info("If you're sure the UI doesn't depend on them, pass --force.")
        return False

    npm_cmd = "npm.cmd" if os.name == "nt" else "npm"
    npx_cmd = "npx.cmd" if os.name == "nt" else "npx"
    if not run_cmd([npx_cmd, "tsc", "--noEmit", "-p", "."], cwd=RUST_DIR):
        log_error("TypeScript errors: not shipping a broken UI.")
        return False
    if not run_cmd([npm_cmd, "run", "build"], cwd=RUST_DIR):
        return False

    build = int(datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S"))
    UI_PATCH_DIR.mkdir(parents=True, exist_ok=True)
    for old in UI_PATCH_DIR.iterdir():
        old.unlink()
    zip_name = f"ui-{version}-{build}.zip"
    zip_path = UI_PATCH_DIR / zip_name
    dist = RUST_DIR / "dist"
    meta = {"app_version": version, "build": build, "notes": notes}
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
        for f in sorted(dist.rglob("*")):
            if f.is_file():
                z.write(f, f.relative_to(dist).as_posix())
        z.writestr("serica-ui.json", json.dumps(meta))

    env = os.environ.copy()
    env["TAURI_SIGNING_PRIVATE_KEY"] = UPDATER_KEY.read_text(encoding="utf-8").strip()
    env.setdefault("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "")
    if not run_cmd([npx_cmd, "tauri", "signer", "sign", str(zip_path)], cwd=RUST_DIR, env=env):
        log_error("Signing failed.")
        return False
    sig_path = zip_path.with_name(zip_name + ".sig")
    signature = sig_path.read_text(encoding="utf-8").strip()
    sig_path.unlink()

    tag = _git("describe", "--tags", "--abbrev=0").stdout.strip()
    gh = subprocess.run(["gh", "release", "view", "--repo", GITHUB_REPO, "--json", "tagName", "-q", ".tagName"],
                        capture_output=True, text=True)
    if gh.returncode == 0 and gh.stdout.strip():
        tag = gh.stdout.strip()  # the release marked Latest, which installs poll
    manifest = {
        "app_version": version,
        "build": build,
        "notes": notes,
        "url": f"https://github.com/{GITHUB_REPO}/releases/download/{tag}/{zip_name}",
        "signature": signature,
        "sha256": hashlib.sha256(zip_path.read_bytes()).hexdigest(),
    }
    manifest_path = UI_PATCH_DIR / "ui-latest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    size_kb = round(zip_path.stat().st_size / 1024)
    log_success(f"UI patch {build} for v{version} ({size_kb} KB) staged in {UI_PATCH_DIR}")

    if not publish:
        log_info(f"Publish with: gh release upload {tag} \"{zip_path}\" \"{manifest_path}\" --clobber --repo {GITHUB_REPO}")
        log_info("(zip first, ui-latest.json last) or rerun with --publish.")
        return True

    # Zip before manifest, so no install ever sees a manifest pointing at nothing.
    for f in (zip_path, manifest_path):
        if not run_cmd(["gh", "release", "upload", tag, str(f), "--clobber", "--repo", GITHUB_REPO], cwd=ROOT_DIR):
            return False
    assets = subprocess.run(["gh", "release", "view", tag, "--repo", GITHUB_REPO, "--json", "assets", "-q", ".assets[].name"],
                            capture_output=True, text=True).stdout.split()
    for name in assets:
        if name.startswith("ui-") and name.endswith(".zip") and name != zip_name:
            subprocess.run(["gh", "release", "delete-asset", tag, name, "-y", "--repo", GITHUB_REPO])
    log_success(f"Published on {tag}. Running copies of v{version} pick it up within 30 minutes (or on next launch).")
    return True


# ==============================================================================
# Publish a staged full release (release/github) to GitHub
# ==============================================================================

def publish_release(live: bool) -> bool:
    """Uploads what --bundle staged in release/github to the GitHub release
    v<version>. Created as a draft (nothing reaches users until you publish
    it on GitHub) unless `live`, which publishes and marks it Latest, so
    installed copies offer the update within their next check."""
    log_header("PUBLISHING RELEASE TO GITHUB")
    version = get_rust_version()
    tag = f"v{version}"
    latest = GITHUB_RELEASE_DIR / "latest.json"
    if not latest.exists():
        log_error(f"Nothing staged in {GITHUB_RELEASE_DIR}. Run: python build.py -b patch --bundle")
        return False
    staged_ver = json.loads(latest.read_text(encoding="utf-8")).get("version")
    if staged_ver != version:
        log_error(f"Staged assets are v{staged_ver}, but the code is v{version}. Rebuild with --bundle.")
        return False

    # The tag must point at the code that was built: committed and pushed.
    if _git("status", "--porcelain", "--untracked-files=no").stdout.strip():
        log_error("Uncommitted changes. Commit the version bump (and anything else in this build) first:")
        log_info(f'  git commit -am "Version {version}" && git push')
        return False
    head = _git("rev-parse", "HEAD").stdout.strip()
    upstream = _git("rev-parse", "@{u}")
    if upstream.returncode != 0 or upstream.stdout.strip() != head:
        log_error("This commit isn't pushed (or the branch has no upstream). Push it first: git push")
        return False

    files = [str(p) for p in sorted(GITHUB_RELEASE_DIR.iterdir()) if p.is_file()]
    exists = subprocess.run(["gh", "release", "view", tag, "--repo", GITHUB_REPO],
                            capture_output=True, text=True).returncode == 0
    if exists:
        log_info(f"Release {tag} exists: replacing its assets.")
        if not run_cmd(["gh", "release", "upload", tag, *files, "--clobber", "--repo", GITHUB_REPO], cwd=ROOT_DIR):
            return False
    else:
        # Always created as a draft first, so latest.json is never public
        # before the installer it points at has finished uploading.
        if not run_cmd(["gh", "release", "create", tag, *files, "--repo", GITHUB_REPO, "--draft",
                        "--title", f"Serica {tag}", "--target", head, "--generate-notes"], cwd=ROOT_DIR):
            return False
    if live:
        if not run_cmd(["gh", "release", "edit", tag, "--repo", GITHUB_REPO, "--draft=false", "--latest"], cwd=ROOT_DIR):
            return False
        log_success(f"{tag} is live. Installed copies will offer the update on their next check.")
    else:
        log_success(f"{tag} uploaded as a draft. Review and publish it on GitHub, or rerun with --live:")
        log_info(f"  https://github.com/{GITHUB_REPO}/releases")
    return True


# ==============================================================================
# Test run and dev mode: an isolated copy you can try without shipping anything
# ==============================================================================

TEST_CONFIG = RUST_DIR / "src-tauri" / "tauri.test.conf.json"
TEST_TARGET_DIR = RUST_DIR / "src-tauri" / "target-test"
TEST_RELEASE_DIR = RELEASE_DIR / "test"


def _test_identifier() -> str:
    return json.loads(TEST_CONFIG.read_text(encoding="utf-8"))["identifier"]


def _test_profile_dirs() -> list[Path]:
    ident = _test_identifier()
    return [Path(os.environ[v]) / ident for v in ("APPDATA", "LOCALAPPDATA") if os.environ.get(v)]


def build_test(fresh: bool, launch: bool = True) -> bool:
    """Builds an optimized copy of the current code as "Serica Test"
    (release/test/Serica-Test.exe) and opens it.

    It is kept apart from the real browser: its own identifier
    (tauri.test.conf.json), so its own profile folder and its own
    single-instance lock (it opens even while Serica is running); no auto
    updates and no UI patches (debug assertions are on, which both check), so
    it always shows exactly the code you built. A log window opens next to it.
    The version, release/rust and the release manifest are left alone."""
    log_header("TEST RUN")
    npm_cmd = "npm.cmd" if os.name == "nt" else "npm"
    log_info("Building the UI...")
    if not run_cmd([npm_cmd, "run", "build"], cwd=RUST_DIR):
        return False

    env = os.environ.copy()
    env["TAURI_CONFIG"] = TEST_CONFIG.read_text(encoding="utf-8")
    env["CARGO_TARGET_DIR"] = str(TEST_TARGET_DIR)  # keeps the real release build's cache intact
    env["CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS"] = "true"
    # Optimized like a release, minus the slow whole-program passes.
    env["CARGO_PROFILE_RELEASE_LTO"] = "false"
    env["CARGO_PROFILE_RELEASE_CODEGEN_UNITS"] = "16"
    cargo_bin = shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / ("cargo.exe" if os.name == "nt" else "cargo"))
    log_info("Building the test copy (the first one takes a while; later ones are quick)...")
    if not run_cmd([cargo_bin, "build", "--release", "--features", "custom-protocol"], cwd=RUST_DIR / "src-tauri", env=env):
        return False

    exe_name = "serica-rust.exe" if os.name == "nt" else "serica-rust"
    built = TEST_TARGET_DIR / "release" / exe_name
    TEST_RELEASE_DIR.mkdir(parents=True, exist_ok=True)
    target = TEST_RELEASE_DIR / ("Serica-Test.exe" if os.name == "nt" else "serica-test")
    try:
        shutil.copy2(built, target)
    except PermissionError:
        log_error(f"{target.name} is still running. Close the test copy and run this again.")
        return False

    if fresh:
        for d in _test_profile_dirs():
            if d.exists():
                log_info(f"Clearing test profile: {d}")
                shutil.rmtree(d, ignore_errors=True)

    log_success(f"Test copy: {BOLD}{target}{RESET}")
    log_info(f"Its profile: {', '.join(str(d) for d in _test_profile_dirs())}")
    if launch:
        subprocess.Popen([str(target)], cwd=str(TEST_RELEASE_DIR))
        log_success("Opened Serica Test.")
    return True


def run_dev() -> bool:
    """`tauri dev` with hot reload, under the test identifier so it uses the
    test profile and runs alongside an installed Serica."""
    log_header("DEV MODE (hot reload, Ctrl+C to stop)")
    npx_cmd = "npx.cmd" if os.name == "nt" else "npx"
    return run_cmd([npx_cmd, "tauri", "dev", "--config", str(TEST_CONFIG.relative_to(RUST_DIR))], cwd=RUST_DIR)


# ==============================================================================
# Interactive menu (no arguments)
# ==============================================================================

def menu_args() -> list[str] | None:
    options = [
        ("Test run: build an isolated test copy and open it", ["--test"]),
        ("Test run with an empty test profile", ["--test", "--fresh"]),
        ("Dev mode: hot reload while editing", ["--dev"]),
        ("Local build: release/rust/Serica.exe", []),
        ("Full release: bump the version, build the signed installer + auto-update files", None),
        ("Publish the staged full release to GitHub (as a draft)", ["--publish-release"]),
        ("UI patch: ship a React/CSS fix without a reinstall", None),
        ("Status: what's built and staged", ["--status"]),
    ]
    print(f"{BOLD}{MAGENTA}Serica build{RESET}  (python build.py --help lists every flag)\n")
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


# ==============================================================================
# Release Directory Summary & Launcher
# ==============================================================================

def print_summary():
    log_header("RELEASE STATUS SUMMARY")
    manifest = load_manifest()
    releases = manifest.get("releases", {})

    if not releases:
        log_warn("No releases recorded in manifest.")
        return

    print(f"{'Environment':<14} {'Version':<10} {'Size (MB)':<12} {'Executable Path'}")
    print("-" * 70)
    for env_name, info in releases.items():
        ver = info.get("version", "unknown")
        size = info.get("size_mb", 0)
        exe = info.get("executable", "unknown")
        full_path = ROOT_DIR / exe
        status_marker = f"{GREEN}✓ Ready{RESET}" if full_path.exists() else f"{RED}✗ Missing{RESET}"
        print(f"{BOLD}{env_name.capitalize():<14}{RESET} {ver:<10} {size:<12} {exe} [{status_marker}]")

    print(f"\n{BOLD}Release Directory:{RESET} {RELEASE_DIR}")
    print_staged()


def print_staged():
    """Test copy, staged full release and staged UI patch, if any."""
    test_exe = TEST_RELEASE_DIR / "Serica-Test.exe"
    if test_exe.exists():
        built = datetime.fromtimestamp(test_exe.stat().st_mtime).strftime("%Y-%m-%d %H:%M")
        print(f"{BOLD}Test copy:{RESET}         {test_exe} (built {built})")
    latest = GITHUB_RELEASE_DIR / "latest.json"
    if latest.exists():
        v = json.loads(latest.read_text(encoding="utf-8")).get("version")
        print(f"{BOLD}Staged release:{RESET}    v{v} in {GITHUB_RELEASE_DIR} (upload: --publish-release)")
    ui = UI_PATCH_DIR / "ui-latest.json"
    if ui.exists():
        m = json.loads(ui.read_text(encoding="utf-8"))
        print(f"{BOLD}Staged UI patch:{RESET}   build {m.get('build')} for v{m.get('app_version')}: {m.get('notes') or '(no notes)'}")


def launch_release(env_name: str):
    manifest = load_manifest()
    releases = manifest.get("releases", {})
    if env_name not in releases:
        log_error(f"Cannot launch '{env_name}': not found in release manifest.")
        return False
    exe_path = ROOT_DIR / releases[env_name]["executable"]
    if not exe_path.exists():
        log_error(f"Executable does not exist: {exe_path}")
        return False
    log_info(f"Launching {env_name} release: {exe_path}")
    env = os.environ.copy()
    subprocess.Popen([str(exe_path)], env=env)
    return True


# ==============================================================================
# CLI Entrypoint
# ==============================================================================

def main():
    parser = argparse.ArgumentParser(
        description="Serica Browser - build, try out and ship. Run with no arguments for a menu.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(__doc__ or "").split("menu.\n", 1)[-1],
    )
    try_out = parser.add_argument_group("try it out (nothing is shipped)")
    try_out.add_argument("--test", action="store_true",
                         help="Test run: build an isolated copy (own profile, no auto-updates) and open it")
    try_out.add_argument("--fresh", action="store_true", help="With --test: start from an empty test profile")
    try_out.add_argument("--no-launch", action="store_true", help="With --test: build only, don't open it")
    try_out.add_argument("--dev", action="store_true", help="Dev mode: tauri dev with hot reload, on the test profile")

    build = parser.add_argument_group("build and full release")
    build.add_argument("-b", "--bump", choices=["major", "minor", "patch", "none"], default="none",
                       help="Version bump increment. Default: none")
    build.add_argument("--clean", action="store_true", help="Clean previous build artifacts before building")
    build.add_argument("--bundle", action="store_true",
                       help="Also build the signed MSI and stage GitHub release assets (with latest.json for auto-update) in release/github")
    build.add_argument("--publish-release", action="store_true",
                       help="Upload what --bundle staged to the GitHub release v<version> (a draft unless --live)")
    build.add_argument("--live", action="store_true", help="With --publish-release: publish it and mark it Latest right away")
    build.add_argument("--run", action="store_true", help="Launch the local build afterwards (uses your real profile)")

    patch = parser.add_argument_group("UI patch (no reinstall)")
    patch.add_argument("--ui-patch", action="store_true",
                       help="Ship only the UI (React/CSS) as a signed patch for the current version. No cargo, no installer.")
    patch.add_argument("--notes", default="", help="With --ui-patch: one line shown to users, e.g. \"AI tabs keep your chat\"")
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
        return 0 if build_test(args.fresh, launch=not args.no_launch) else 1
    if args.dev:
        return 0 if run_dev() else 1
    if args.ui_patch:
        return 0 if build_ui_patch(args.notes, args.publish, args.force) else 1
    if args.publish_release:
        return 0 if publish_release(args.live) else 1

    print(f"{BOLD}{MAGENTA}================================================================{RESET}")
    print(f"{BOLD}{MAGENTA}              SERICA BROWSER - BUILD SYSTEM                   {RESET}")
    print(f"{BOLD}{MAGENTA}================================================================{RESET}")
    log_info(f"Version Bump Mode : {BOLD}{args.bump.upper()}{RESET}")
    log_info(f"Clean Build       : {BOLD}{args.clean}{RESET}")

    if not build_rust(args.bump, args.clean):
        log_error("Build failed.")
        return 1

    if args.bundle:
        if not build_bundle():
            log_error("Bundling failed.")
            return 1
        version = get_rust_version()
        log_info("Next: commit and push the version bump, then upload the release:")
        log_info(f'  git commit -am "Version {version}" && git push')
        log_info("  python build.py --publish-release        (a draft; add --live to publish at once)")

    print_summary()
    log_success("Build completed successfully!")
    if args.run:
        launch_release("rust")
    return 0


if __name__ == "__main__":
    sys.exit(main())
