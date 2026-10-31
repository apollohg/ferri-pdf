#!/usr/bin/env python3
"""Audit the library's resolved dependency graph and macOS CLI binary."""
import argparse
import hashlib
import json
import platform
import re
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
NATIVE_BUILD_CRATES = {"cc", "cmake", "pkg-config", "bindgen", "vcpkg"}
PLATFORM_LIBRARIES = {"/usr/lib/libSystem.B.dylib", "/usr/lib/libiconv.2.dylib"}
# Rayon uses Cargo links solely to enforce a single Rust runtime version.
RUST_LINK_GUARDS = {
    ("rayon-core", "1.13.0", "rayon-core"): "fa31cb198b772600d100a7c403ddedccef637d2e6b2da431fa7f02ca41307fc6",
}
# rav1d's build script, and its cc and nasm-rs build tools, are inert when only Rust bit-depth features are enabled.
RUST_ONLY_BUILD_FEATURES = {
    ("ferri-pdf-rav1d", "1.1.1"): ("a7b17c3c9498d98835197a9654b1fe1f41f745e62e9fc9bf2fd27f7840a68c58", {"bitdepth_8", "bitdepth_16"}),
}
NATIVE_BUILD_MARKERS = ("cc::Build", "cmake::", "pkg_config::", "vcpkg::")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/ferri-pdf")
    arguments = parser.parse_args()
    target = subprocess.check_output(["rustc", "-vV"], text=True).split("host: ")[1].splitlines()[0]
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--offline", "--locked", "--format-version", "1",
        "--filter-platform", target,
        "--manifest-path", str(ROOT / "Cargo.toml"),
    ]))
    # Metadata includes inactive optional dependencies retained in Cargo.lock.
    tree = subprocess.check_output([
        "cargo", "tree", "--offline", "--locked", "--workspace", "--target", target,
        "--edges", "normal,build,dev", "--prefix", "none", "--format", "{p}|{f}",
        "--manifest-path", str(ROOT / "Cargo.toml"),
    ], text=True)
    active_features = {}
    for name, version, features in re.findall(r"^(\S+) v([^\s|]+)[^|\n]*\|([^ \n]*)", tree, re.MULTILINE):
        active_features.setdefault((name, version), set()).update(filter(None, features.split(",")))
    active = set(active_features)
    identities = {package["id"]: (package["name"], package["version"]) for package in metadata["packages"]}
    dependents = {}
    for node in metadata["resolve"]["nodes"]:
        for dependency in node["deps"]:
            dependents.setdefault(identities[dependency["pkg"]], set()).add(identities[node["id"]])
    rust_only_builds = set()
    for package in metadata["packages"]:
        identity = (package["name"], package["version"])
        feature_guard = RUST_ONLY_BUILD_FEATURES.get(identity)
        if identity not in active or feature_guard is None:
            continue
        builds = [Path(target["src_path"]) for target in package["targets"] if "custom-build" in target["kind"]]
        if all(hashlib.sha256(path.read_bytes()).hexdigest() == feature_guard[0] for path in builds) and active_features[identity].issubset(feature_guard[1]):
            rust_only_builds.add(identity)
    inactive = []
    violations = []
    packages = []
    for package in metadata["packages"]:
        if (package["name"], package["version"]) not in active:
            inactive.append({"name": package["name"], "version": package["version"]})
            continue
        scripts = []
        for target in package["targets"]:
            if "custom-build" not in target["kind"]:
                continue
            path = Path(target["src_path"])
            source = path.read_text()
            source_digest = hashlib.sha256(path.read_bytes()).hexdigest()
            for marker in NATIVE_BUILD_MARKERS:
                if marker in source and (package["name"], package["version"]) not in rust_only_builds:
                    violations.append(f"native build marker {marker} in {path}")
            scripts.append({"file": path.name, "sha256": source_digest})
        guard = RUST_LINK_GUARDS.get((package["name"], package["version"], package.get("links")))
        verified_rust_guard = guard is not None and scripts == [{"file": "build.rs", "sha256": guard}]
        dependent_builds = dependents.get((package["name"], package["version"]), set()) & active
        unused_native_tool = package["name"] in NATIVE_BUILD_CRATES and dependent_builds and dependent_builds <= rust_only_builds
        if (package["name"] in NATIVE_BUILD_CRATES and not unused_native_tool) or (package.get("links") and not verified_rust_guard):
            violations.append(f"native build/link candidate: {package['name']}")
        manifest = Path(package["manifest_path"])
        source = package["source"] or str(manifest.parent.relative_to(ROOT) if manifest.is_relative_to(ROOT) else manifest.parent)
        packages.append({"name": package["name"], "version": package["version"], "source": source, "manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(), "license": package["license"], "features": sorted(active_features[(package["name"], package["version"])]), "links": package.get("links"), "build_scripts": scripts})
    libraries = []
    imported_rendering_symbols = []
    binary = arguments.binary.resolve()
    if platform.system() == "Darwin":
        linked = subprocess.check_output(["otool", "-L", str(binary)], text=True)
        libraries = [line.strip().split(" (", 1)[0] for line in linked.splitlines()[1:]]
        violations.extend(f"unexpected linked library: {name}" for name in libraries if name not in PLATFORM_LIBRARIES)
        symbols = subprocess.check_output(["nm", "-u", str(binary)], text=True)
        imported_rendering_symbols = [line for line in symbols.splitlines() if any(name in line.lower() for name in ("iconv", "freetype", "cairo", "harfbuzz", "coretext", "cgcontext"))]
        violations.extend(imported_rendering_symbols)
    report = {"binary": str(binary.relative_to(ROOT) if binary.is_relative_to(ROOT) else binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "packages": packages, "inactive_lock_packages": inactive, "linked_libraries": libraries, "native_rendering_imports": imported_rendering_symbols, "violations": violations, "scope": "Cargo tree current-target workspace normal/build/dev dependencies, native build-tool/links metadata, build scripts, and macOS dynamic imports. Inactive optional lockfile packages are listed separately. This covers the library and CLI configuration, not all optional crate features or platforms."}
    (HERE / "output").mkdir(exist_ok=True)
    (HERE / "output/dependency-audit.json").write_text(json.dumps(report, indent=2))
    print(f"Audited {len(packages)} packages; {len(violations)} violations")
    if violations:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
