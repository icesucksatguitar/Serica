#!/usr/bin/env python3
"""
==============================================================================
SERICA BROWSER - BUILD & RELEASE SCRIPT
==============================================================================
Builds, versions, and stages the Rust (Tauri v2) release:

  release/
  ├── rust/            <- Standalone Windows executable (Serica.exe) + icons
  └── manifest.json    <- Current version, size and build time

Usage:
  python build.py --bump [major|minor|patch|none] [--clean] [--run]
  python build.py -b patch
  python build.py -b patch --bundle   <- release: signed MSI + latest.json in release/github
  python build.py --ui-patch --notes "Fixed X" --publish
                                      <- UI-only fix: signed in ~30 s, live for users in minutes
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
        description="Serica Browser - build and stage the Rust (Tauri v2) release",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python build.py --bump none
  python build.py --bump patch --clean
  python build.py --run
        """
    )
    parser.add_argument(
        "-b", "--bump",
        choices=["major", "minor", "patch", "none"],
        default="none",
        help="Version bump increment (major, minor, patch, none). Default: none"
    )
    parser.add_argument(
        "--clean",
        action="store_true",
        help="Clean previous build artifacts before building"
    )
    parser.add_argument(
        "--bundle",
        action="store_true",
        help="Also build the signed MSI and stage GitHub release assets (with latest.json for auto-update) in release/github"
    )
    parser.add_argument(
        "--ui-patch",
        action="store_true",
        help="Ship only the UI (React/CSS) as a signed patch for the current version. No cargo, no installer."
    )
    parser.add_argument("--notes", default="", help="With --ui-patch: one line shown to users, e.g. \"AI tabs keep your chat\"")
    parser.add_argument("--publish", action="store_true", help="With --ui-patch: upload it to the GitHub release")
    parser.add_argument("--force", action="store_true", help="With --ui-patch: ship even though Rust code changed")
    parser.add_argument(
        "--run",
        action="store_true",
        help="Launch the application after a successful build"
    )

    args = parser.parse_args()

    if args.ui_patch:
        return 0 if build_ui_patch(args.notes, args.publish, args.force) else 1

    print(f"{BOLD}{MAGENTA}================================================================{RESET}")
    print(f"{BOLD}{MAGENTA}              SERICA BROWSER - BUILD SYSTEM                   {RESET}")
    print(f"{BOLD}{MAGENTA}================================================================{RESET}")
    log_info(f"Version Bump Mode : {BOLD}{args.bump.upper()}{RESET}")
    log_info(f"Clean Build       : {BOLD}{args.clean}{RESET}")

    if not build_rust(args.bump, args.clean):
        log_error("Build failed.")
        return 1

    if args.bundle and not build_bundle():
        log_error("Bundling failed.")
        return 1

    print_summary()
    log_success("Build completed successfully!")
    if args.run:
        launch_release("rust")
    return 0


if __name__ == "__main__":
    sys.exit(main())
