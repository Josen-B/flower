"""Flower 打包器测试。

测试使用临时仓库、注入的 runner 和合成 PE，只验证内部组包入口，不把测试
产物当作真实发布包。
"""

from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import shutil
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = REPO_ROOT / "scripts" / "package.py"
SPEC = importlib.util.spec_from_file_location("flower_package", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("无法加载 scripts/package.py")
flower_package = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = flower_package
SPEC.loader.exec_module(flower_package)


MIT_TEXT = (
    "MIT License\n\n"
    "Permission is hereby granted, free of charge, to any person obtaining a "
    "copy of this software.\n\n"
    'THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.\n'
)
APACHE_TEXT = (
    "Apache License\nVersion 2.0, January 2004\n\n"
    "Licensed under the Apache License, Version 2.0.\n"
)
UNLICENSE_TEXT = (
    "This is free and unencumbered software released into the public domain.\n"
)
UNICODE_TEXT = "UNICODE LICENSE V3\nCopyright © 1991-2024 Unicode, Inc.\n"
RELEASE_PROFILE = {
    "opt_level": "3",
    "debug_assertions": False,
    "test": False,
}
DEBUG_PROFILE = {
    "opt_level": "0",
    "debug_assertions": True,
    "test": False,
}


def _u16(data: bytearray, offset: int, value: int) -> None:
    data[offset : offset + 2] = value.to_bytes(2, "little")


def _u32(data: bytearray, offset: int, value: int) -> None:
    data[offset : offset + 4] = value.to_bytes(4, "little")


def make_pe(*, is_dll: bool, exports: set[str] | None = None) -> bytes:
    exports = (
        set(flower_package.EXPECTED_DLL_EXPORTS)
        if exports is None
        else set(exports)
    )
    data = bytearray(0x600)
    data[0:2] = b"MZ"
    pe_offset = 0x80
    _u32(data, 0x3C, pe_offset)
    data[pe_offset : pe_offset + 4] = b"PE\0\0"

    coff = pe_offset + 4
    _u16(data, coff, 0x8664)
    _u16(data, coff + 2, 1)
    _u16(data, coff + 16, 0xF0)
    _u16(data, coff + 18, 0x2002 if is_dll else 0x0022)

    optional = coff + 20
    _u16(data, optional, 0x20B)
    _u32(data, optional + 56, 0x2000)
    _u32(data, optional + 60, 0x200)
    _u32(data, optional + 108, 16)

    section = optional + 0xF0
    _u32(data, section + 8, 0x400)
    _u32(data, section + 12, 0x1000)
    _u32(data, section + 16, 0x400)
    _u32(data, section + 20, 0x200)

    if not is_dll:
        return bytes(data)

    export_rva = 0x1000
    _u32(data, optional + 112, export_rva)
    _u32(data, optional + 116, 0x100)
    export_offset = 0x200
    _u32(data, export_offset + 12, 0x1028)
    _u32(data, export_offset + 20, len(exports))
    _u32(data, export_offset + 24, len(exports))
    functions_rva = 0x1034
    names_rva = 0x103C
    ordinals_rva = 0x1044
    _u32(data, export_offset + 28, functions_rva)
    _u32(data, export_offset + 32, names_rva)
    _u32(data, export_offset + 36, ordinals_rva)
    data[0x228:0x233] = b"flower.dll\0"

    for index, export in enumerate(sorted(exports)):
        _u32(data, 0x234 + index * 4, 0x1100 + index * 0x10)
    string_offset = 0x248
    for index, export in enumerate(sorted(exports)):
        encoded = export.encode("ascii") + b"\0"
        string_rva = export_rva + (string_offset - export_offset)
        _u32(data, 0x23C + index * 4, string_rva)
        _u16(data, 0x244 + index * 2, index)
        data[string_offset : string_offset + len(encoded)] = encoded
        string_offset += len(encoded)
    return bytes(data)


class FakeRunner:
    def __init__(
        self,
        *,
        metadata: dict,
        sysroot: Path,
        artifacts: list[tuple[str, Path]],
        build_exit: int = 0,
        metadata_exit: int = 0,
        extra_artifacts: list[dict] | None = None,
        relative_artifacts: bool = False,
        artifact_profile: dict | None = None,
    ) -> None:
        self.metadata = metadata
        self.sysroot = sysroot
        self.artifacts = artifacts
        self.build_exit = build_exit
        self.metadata_exit = metadata_exit
        self.extra_artifacts = extra_artifacts or []
        self.relative_artifacts = relative_artifacts
        self.artifact_profile = (
            RELEASE_PROFILE if artifact_profile is None else artifact_profile
        )
        self.calls: list[tuple[list[str], Path]] = []

    def __call__(self, args, cwd):
        command = list(args)
        self.calls.append((command, Path(cwd)))
        if command[:2] == ["cargo", "metadata"]:
            return subprocess_result(
                command,
                self.metadata_exit,
                stdout=json.dumps(self.metadata)
                if self.metadata_exit == 0
                else "",
                stderr="" if self.metadata_exit == 0 else "metadata failed",
            )
        if command[:2] == ["cargo", "build"]:
            if self.build_exit != 0:
                return subprocess_result(
                    command, self.build_exit, stderr="build failed"
                )
            lines = list(self.extra_artifacts)
            for target_name, path in self.artifacts:
                emitted_path = (
                    path.relative_to(Path(self.metadata["target_directory"])).as_posix()
                    if self.relative_artifacts
                    else str(path)
                )
                if target_name == "flower":
                    lines.append(
                        {
                            "reason": "compiler-artifact",
                            "package_id": self.metadata["resolve"]["root"],
                            "target": {
                                "kind": ["lib"],
                                "crate_types": ["cdylib", "rlib"],
                                "name": "flower",
                            },
                            "profile": dict(self.artifact_profile),
                            "filenames": [emitted_path, emitted_path + ".rlib"],
                            "executable": None,
                        }
                    )
                else:
                    lines.append(
                        {
                            "reason": "compiler-artifact",
                            "package_id": self.metadata["resolve"]["root"],
                            "target": {"kind": ["bin"], "name": target_name},
                            "profile": dict(self.artifact_profile),
                            "filenames": [emitted_path],
                            "executable": emitted_path,
                        }
                    )
            return subprocess_result(
                command,
                0,
                stdout="\n".join(json.dumps(line) for line in lines) + "\n",
            )
        if command == ["rustc", "-Vv"]:
            return subprocess_result(
                command,
                0,
                stdout=(
                    "rustc 1.98.0 (test 2026-01-01)\n"
                    "binary: rustc\n"
                    "commit-hash: 0123456789abcdef\n"
                    "host: x86_64-pc-windows-msvc\n"
                    f"release: 1.98.0\n"
                ),
            )
        if command == ["rustc", "--print", "sysroot"]:
            return subprocess_result(command, 0, stdout=str(self.sysroot) + "\n")
        raise AssertionError(f"未预期的测试命令：{command}")


def subprocess_result(
    args: list[str], returncode: int, stdout: str = "", stderr: str = ""
):
    return flower_package.subprocess.CompletedProcess(
        args, returncode, stdout=stdout, stderr=stderr
    )


class PackageFixture:
    def __init__(self, base: Path) -> None:
        self.base = base
        self.repo = base / "repo with spaces"
        self.repo.mkdir()
        (self.repo / "docs").mkdir()
        (self.repo / "data" / "pinyin").mkdir(parents=True)
        (self.repo / "docs" / "distribution.md").write_text(
            "# 测试包说明\n\n这是临时测试说明。\n", encoding="utf-8"
        )
        (self.repo / "data" / "pinyin" / "mvp_lexicon.tsv").write_text(
            "ni\t你\t10\n", encoding="utf-8"
        )
        # Real, byte-locked dictionary bundle, but the PE binaries remain synthetic.
        for name in (
            "base_lexicon.tsv", "coverage.json", "sources/manifest.json",
            "sources/kMandarin_8105.txt", "sources/UNICODE-LICENSE.txt",
            "sources/LICENSE-pinyin-data.txt", "sources/NOTICE.md",
        ):
            destination = self.repo / "data/pinyin" / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO_ROOT / "data/pinyin" / name, destination)
        shutil.copyfile(REPO_ROOT / "LICENSE-MIT", self.repo / "LICENSE-MIT")
        for name in (
            "phrase_lexicon.tsv", "proper_lexicon.tsv", "coverage.json", "manifest.json",
            "quality-cases.json", "NOTICE.md", "sources/CC-BY-SA-4.0.txt", "sources/LICENSE-jieba.txt",
            "sources/dict.txt",
        ):
            destination = self.repo / "data/pinyin/phrases" / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO_ROOT / "data/pinyin/phrases" / name, destination)
        shutil.copyfile(REPO_ROOT / "LICENSE-APACHE", self.repo / "LICENSE-APACHE")
        for name in ("everyday_phrases.tsv", "everyday_phrases.md"):
            shutil.copyfile(REPO_ROOT / "data/pinyin" / name,
                            self.repo / "data/pinyin" / name)
        (self.repo / "Cargo.lock").write_text("# test lock\n", encoding="utf-8")

        self.target_dir = base / "custom target dir"
        self.release_dir = (
            self.target_dir / flower_package.TARGET / "release"
        )
        self.release_dir.mkdir(parents=True)
        self.dll = self.release_dir / "flower.dll"
        self.settings = self.release_dir / "flower-settings.exe"
        self.register = self.release_dir / "flower-register.exe"
        self.dll.write_bytes(make_pe(is_dll=True))
        self.settings.write_bytes(make_pe(is_dll=False))
        self.register.write_bytes(make_pe(is_dll=False))

        self.sysroot = base / "fake sysroot"
        rust_docs = self.sysroot / "share" / "doc" / "rust"
        (rust_docs / "licenses").mkdir(parents=True)
        (rust_docs / "COPYRIGHT-library.html").write_text(
            "<h1>Rust Standard Library copyright</h1>\n", encoding="utf-8"
        )
        (rust_docs / "COPYRIGHT.html").write_text(
            "<h1>Rust copyright</h1>\n", encoding="utf-8"
        )
        (rust_docs / "licenses" / "MIT.txt").write_text(
            MIT_TEXT, encoding="utf-8"
        )
        (rust_docs / "licenses" / "Apache-2.0.txt").write_text(
            APACHE_TEXT, encoding="utf-8"
        )
        (rust_docs / "licenses" / "Unicode-3.0.txt").write_text(
            UNICODE_TEXT, encoding="utf-8"
        )

        self.root_id = "path+file:///fixture#flower@0.1.0"
        self.dependencies = [
            self._dependency("alpha", "1.0.0", "MIT OR Apache-2.0"),
            self._dependency("beta", "1.0.0", "MIT"),
            self._dependency(
                "gamma",
                "1.0.0",
                "Unlicense OR MIT",
                file_names=("LICENSE-MIT", "UNLICENSE"),
            ),
            self._dependency(
                "unicode-ident",
                "1.0.27",
                "(MIT OR Apache-2.0) AND Unicode-3.0",
                file_names=("LICENSE-MIT", "LICENSE-APACHE", "LICENSE-UNICODE"),
            ),
        ]
        self.metadata = self._metadata("0.1.0")

    def _dependency(
        self,
        name: str,
        version: str,
        expression: str,
        *,
        file_names: tuple[str, ...] = ("LICENSE-MIT", "LICENSE-APACHE"),
    ) -> dict:
        directory = self.repo / "vendor" / f"{name}-{version}"
        directory.mkdir(parents=True)
        (directory / "Cargo.toml").write_text(
            f'[package]\nname = "{name}"\nversion = "{version}"\n',
            encoding="utf-8",
        )
        texts = {
            "LICENSE-MIT": MIT_TEXT,
            "LICENSE-APACHE": APACHE_TEXT,
            "UNLICENSE": UNLICENSE_TEXT,
            "LICENSE-UNICODE": UNICODE_TEXT,
        }
        for file_name in file_names:
            (directory / file_name).write_text(
                texts[file_name], encoding="utf-8"
            )
        package_id = f"registry+fixture#{name}@{version}"
        return {
            "id": package_id,
            "name": name,
            "version": version,
            "license": expression,
            "repository": f"https://example.invalid/{name}",
            "source": "registry+https://example.invalid/index",
            "manifest_path": str(directory / "Cargo.toml"),
        }

    def _metadata(self, version: str) -> dict:
        root_package = {
            "id": self.root_id,
            "name": "flower",
            "version": version,
            "license": "MIT OR Apache-2.0",
            "repository": "https://example.invalid/flower",
            "source": None,
            "manifest_path": str(self.repo / "Cargo.toml"),
        }
        packages = [root_package, *self.dependencies]
        root_dependencies = [
            {"name": package["name"], "pkg": package["id"], "dep_kinds": []}
            for package in self.dependencies
        ]
        nodes = [
            {"id": self.root_id, "deps": root_dependencies},
            *[
                {"id": package["id"], "deps": []}
                for package in self.dependencies
            ],
        ]
        return {
            "packages": packages,
            "resolve": {"root": self.root_id, "nodes": nodes},
            "target_directory": str(self.target_dir),
        }

    def runner(self, **kwargs) -> FakeRunner:
        return FakeRunner(
            metadata=self.metadata,
            sysroot=self.sysroot,
            artifacts=[
                ("flower", self.dll),
                ("flower-settings", self.settings),
                ("flower-register", self.register),
            ],
            **kwargs,
        )


class PackageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.fixture = PackageFixture(self.base)

    def run_package(self, runner: FakeRunner, output_name: str = "out"):
        output = self.base / output_name
        return flower_package.build_package(
            repo_root=self.fixture.repo,
            output_dir=output,
            host_platform="win32",
            runner=runner,
        )

    def test_dictionary_bundle_rejects_changed_generated_data_and_missing_license(self):
        source = self.fixture.repo / "data/pinyin/base_lexicon.tsv"
        original = source.read_bytes()
        source.write_bytes(original + b"tampered")
        with self.assertRaisesRegex(flower_package.PackagingError, "单字库"):
            flower_package._dictionary_bundle(self.fixture.repo)
        source.write_bytes(original)
        (self.fixture.repo / "data/pinyin/sources/UNICODE-LICENSE.txt").unlink()
        with self.assertRaises(flower_package.PackagingError):
            flower_package._dictionary_bundle(self.fixture.repo)

    def test_dictionary_bundle_rejects_a_forged_coverage_summary(self):
        path = self.fixture.repo / "data/pinyin/coverage.json"
        report = json.loads(path.read_text(encoding="utf-8"))
        report["unique_characters"] += 1
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(flower_package.PackagingError, "实际覆盖"):
            flower_package._dictionary_bundle(self.fixture.repo)

    def test_dictionary_bundle_rejects_forged_ranking_metadata(self):
        path = self.fixture.repo / "data/pinyin/coverage.json"
        report = json.loads(path.read_text(encoding="utf-8"))
        report["ranking"]["revision"] = "0" * 40
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(flower_package.PackagingError, "排序来源"):
            flower_package._dictionary_bundle(self.fixture.repo)

    def test_dictionary_bundle_rejects_changed_ranking_snapshot_or_license(self):
        for name in ("dict.txt", "LICENSE-jieba.txt"):
            path = self.fixture.repo / "data/pinyin/phrases/sources" / name
            original = path.read_bytes()
            path.write_bytes(b"broken")
            with self.assertRaisesRegex(flower_package.PackagingError, "排序来源"):
                flower_package._dictionary_bundle(self.fixture.repo)
            path.write_bytes(original)

    def test_everyday_bundle_rejects_missing_invalid_count_hash_and_notice(self):
        path = self.fixture.repo / "data/pinyin/everyday_phrases.tsv"
        original = path.read_bytes()
        cases = [
            b"bad\tbad\t0\n",
            b"\n".join(original.splitlines()[:-1]) + b"\n",
            original + b"wo'ai'ni\tduplicate\t100\n",
            original.replace(b"\t100", b"\t101", 1),
            original.replace("我爱你".encode(), "我想你".encode(), 1),
        ]
        for data in cases:
            with self.subTest(data=data[:60]):
                path.write_bytes(data)
                with self.assertRaises(flower_package.PackagingError):
                    flower_package._everyday_bundle(self.fixture.repo)
        path.write_bytes(original)
        payload, record = flower_package._everyday_bundle(self.fixture.repo)
        self.assertEqual(record["pairs"], 155)
        self.assertEqual(record["sha256"], hashlib.sha256(original).hexdigest())
        self.assertIn(record["notice_path"], payload)
        path.unlink()
        with self.assertRaises(flower_package.PackagingError):
            flower_package._everyday_bundle(self.fixture.repo)
        path.write_bytes(original)
        notice = self.fixture.repo / "data/pinyin/everyday_phrases.md"
        notice.write_text("missing attribution", encoding="utf-8")
        with self.assertRaises(flower_package.PackagingError):
            flower_package._everyday_bundle(self.fixture.repo)
        notice.unlink()
        with self.assertRaises(flower_package.PackagingError):
            flower_package._everyday_bundle(self.fixture.repo)

    def test_phrase_bundle_rejects_changed_core_or_missing_sharealike_license(self):
        path = self.fixture.repo / "data/pinyin/phrases/phrase_lexicon.tsv"
        original = path.read_bytes()
        path.write_bytes(original + b"broken")
        with self.assertRaisesRegex(flower_package.PackagingError, "词组"):
            flower_package._phrase_bundle(self.fixture.repo)
        path.write_bytes(original)
        (self.fixture.repo / "data/pinyin/phrases/sources/CC-BY-SA-4.0.txt").unlink()
        with self.assertRaises(flower_package.PackagingError):
            flower_package._phrase_bundle(self.fixture.repo)

    def test_help_succeeds_and_real_cli_refuses_non_windows(self) -> None:
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            with self.assertRaises(SystemExit) as raised:
                flower_package.main(["--help"])
        self.assertEqual(raised.exception.code, 0)
        self.assertIn("--output-dir", output.getvalue())
        self.assertIn("用法：", output.getvalue())
        self.assertIn("选项", output.getvalue())

        argument_error = io.StringIO()
        with contextlib.redirect_stderr(argument_error):
            with self.assertRaises(SystemExit) as raised:
                flower_package.main(["--不存在的选项"])
        self.assertEqual(raised.exception.code, 2)
        self.assertIn("无法识别的参数", argument_error.getvalue())

        error = io.StringIO()
        with mock.patch.object(flower_package.sys, "platform", "linux"):
            with contextlib.redirect_stderr(error):
                result = flower_package.main(
                    ["--output-dir", str(self.base / "not-created")]
                )
        self.assertNotEqual(result, 0)
        self.assertIn("只支持 Windows", error.getvalue())
        self.assertFalse((self.base / "not-created").exists())

    def test_fixture_exports_exactly_the_expected_names(self) -> None:
        data = self.fixture.dll.read_bytes()
        expected_names = sorted(flower_package.EXPECTED_DLL_EXPORTS)
        raw_name_offset = 0x248
        for index, name in enumerate(expected_names):
            expected_rva = 0x1000 + raw_name_offset - 0x200
            self.assertEqual(
                int.from_bytes(
                    data[0x23C + index * 4 : 0x240 + index * 4], "little"
                ),
                expected_rva,
            )
            encoded = name.encode("ascii") + b"\0"
            self.assertEqual(
                data[raw_name_offset : raw_name_offset + len(encoded)],
                encoded,
            )
            raw_name_offset += len(encoded)

        image = flower_package._parse_pe(data, "flower.dll")
        self.assertEqual(
            flower_package._export_names(image, data),
            set(expected_names),
        )

    def test_package_has_whitelist_manifest_hashes_and_deterministic_bytes(
        self,
    ) -> None:
        runner = self.fixture.runner()
        result = self.run_package(runner)
        self.assertTrue(result.zip_path.is_file())
        self.assertTrue(result.checksum_path.is_file())
        self.assertEqual(
            runner.calls[0][0],
            [
                "cargo",
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--filter-platform",
                "x86_64-pc-windows-msvc",
            ],
        )
        self.assertEqual(
            runner.calls[1][0],
            [
                "cargo",
                "build",
                "--locked",
                "--release",
                "--lib",
                "--bin",
                "flower-register",
                "--bin",
                "flower-settings",
                "--target",
                "x86_64-pc-windows-msvc",
                "--message-format=json-render-diagnostics",
            ],
        )
        self.assertEqual(runner.calls[0][1], self.fixture.repo)

        with zipfile.ZipFile(result.zip_path) as archive:
            names = archive.namelist()
            self.assertTrue(all(name.startswith(result.zip_path.stem + "/") for name in names))
            relative_names = {
                name.split("/", 1)[1]
                for name in names
            }
            required = {
                "flower.dll",
                "flower-settings.exe",
                "flower-register.exe",
                "README.md",
                "LICENSE-MIT",
                "LICENSE-APACHE",
                "THIRD-PARTY-NOTICES.md",
                "Cargo.lock",
                "manifest.json",
                "SHA256SUMS",
                "data/pinyin/mvp_lexicon.tsv",
                "data/pinyin/everyday_phrases.tsv",
                "data/pinyin/everyday_phrases.md",
                "data/pinyin/base_lexicon.tsv",
                "data/pinyin/coverage.json",
                "data/pinyin/sources/manifest.json",
                "data/pinyin/sources/kMandarin_8105.txt",
                "data/pinyin/phrases/phrase_lexicon.tsv",
                "data/pinyin/phrases/proper_lexicon.tsv",
                "data/pinyin/phrases/coverage.json",
                "data/pinyin/phrases/manifest.json",
                "data/pinyin/phrases/quality-cases.json",
            }
            self.assertTrue(required <= relative_names)
            self.assertEqual(
                {name for name in relative_names if not name.startswith("licenses/")},
                required,
            )
            self.assertFalse(any(
                name.lower().endswith((".pdb", ".rlib"))
                or name.startswith("src/")
                or ".git" in name.split("/")
                for name in relative_names
            ))
            self.assertTrue(any(
                name.startswith("licenses/unicode-ident-1.0.27/")
                for name in relative_names
            ))
            manifest_bytes = archive.read(
                result.zip_path.stem + "/manifest.json"
            )
            manifest = json.loads(manifest_bytes)
            self.assertEqual(manifest["package"], "flower")
            everyday = manifest["everyday_phrases"]
            data = archive.read(result.zip_path.stem + "/" + everyday["path"])
            self.assertEqual(everyday["pairs"], flower_package.EVERYDAY_COUNT)
            self.assertEqual(everyday["sha256"], hashlib.sha256(data).hexdigest())
            self.assertEqual(everyday["license"], "MIT")
            self.assertIn("manual", everyday["weight_kind"])
            self.assertEqual(manifest["version"], "0.1.0")
            self.assertEqual(manifest["target"], "x86_64-pc-windows-msvc")
            self.assertFalse(manifest["distribution"]["authenticode_signed"])
            self.assertFalse(manifest["distribution"]["windows_desktop_verified"])
            self.assertEqual(
                {entry["role"] for entry in manifest["binaries"]},
                {"text_service_dll", "settings_ui", "registration_cli"},
            )
            for entry in manifest["binaries"]:
                binary = archive.read(
                    result.zip_path.stem + "/" + entry["path"]
                )
                self.assertEqual(entry["size"], len(binary))
                self.assertEqual(entry["sha256"], hashlib.sha256(binary).hexdigest())
            self.assertEqual(
                manifest["cargo_lock_sha256"],
                hashlib.sha256(
                    archive.read(result.zip_path.stem + "/Cargo.lock")
                ).hexdigest(),
            )
            self.assertIn("Unicode-3.0", json.dumps(
                manifest["third_party"], ensure_ascii=False
            ))
            self.assertNotIn(str(self.fixture.repo), manifest_bytes.decode("utf-8"))
            self.assertNotIn(str(self.fixture.base), manifest_bytes.decode("utf-8"))
            self.assertNotIn("SHA256SUMS", manifest_bytes.decode("utf-8"))
            self.assertEqual(
                archive.read(result.zip_path.stem + "/README.md"),
                (self.fixture.repo / "docs" / "distribution.md").read_bytes(),
            )

            checksums = archive.read(
                result.zip_path.stem + "/SHA256SUMS"
            ).decode("ascii")
            checksum_names = set()
            for line in checksums.splitlines():
                digest, relative = line.split("  ", 1)
                checksum_names.add(relative)
                self.assertEqual(
                    digest,
                    hashlib.sha256(
                        archive.read(result.zip_path.stem + "/" + relative)
                    ).hexdigest(),
                )
            self.assertEqual(
                checksum_names,
                relative_names - {"SHA256SUMS"},
            )
            notice = archive.read(
                result.zip_path.stem + "/THIRD-PARTY-NOTICES.md"
            ).decode("utf-8")
            self.assertIn("Unicode-3.0", notice)
            self.assertIn("许可证正文", notice)
            self.assertIn("SPDX 标识", notice)

        expected_zip_hash = result.checksum_path.read_text("ascii").split()[0]
        self.assertEqual(
            expected_zip_hash, hashlib.sha256(result.zip_path.read_bytes()).hexdigest()
        )

        second = self.run_package(self.fixture.runner(), "out2")
        self.assertEqual(result.zip_path.read_bytes(), second.zip_path.read_bytes())
        self.assertEqual(
            result.checksum_path.read_text("ascii"),
            second.checksum_path.read_text("ascii"),
        )

    def test_custom_target_and_release_artifact_selection(self) -> None:
        debug_dir = (
            self.fixture.target_dir
            / flower_package.TARGET
            / "debug"
        )
        debug_dir.mkdir()
        debug_dll = debug_dir / "flower.dll"
        debug_dll.write_bytes(make_pe(is_dll=True))
        extra = [
            {
                "reason": "compiler-artifact",
                "package_id": self.fixture.root_id,
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["cdylib", "rlib"],
                    "name": "flower",
                },
                "profile": dict(DEBUG_PROFILE),
                "filenames": [str(debug_dll)],
                "executable": None,
            },
            {
                "reason": "compiler-artifact",
                "package_id": self.fixture.root_id,
                "target": {"kind": ["bin"], "name": "not-flower"},
                "profile": dict(RELEASE_PROFILE),
                "filenames": [str(debug_dll)],
                "executable": str(debug_dll),
            },
            {
                "reason": "compiler-artifact",
                "package_id": "registry+fixture#other@1.0.0",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["cdylib"],
                    "name": "flower",
                },
                "profile": dict(RELEASE_PROFILE),
                "filenames": [str(debug_dll)],
                "executable": None,
            },
        ]
        result = self.run_package(
            self.fixture.runner(
                extra_artifacts=extra, relative_artifacts=True
            )
        )
        with zipfile.ZipFile(result.zip_path) as archive:
            packaged_dll = archive.read(
                result.zip_path.stem + "/flower.dll"
            )
        self.assertEqual(packaged_dll, self.fixture.dll.read_bytes())

    def test_only_nonrelease_root_artifacts_are_rejected(self) -> None:
        debug_dir = (
            self.fixture.target_dir
            / flower_package.TARGET
            / "debug"
        )
        debug_dir.mkdir()
        debug_dll = debug_dir / "flower.dll"
        debug_dll.write_bytes(make_pe(is_dll=True))
        runner = self.fixture.runner(artifact_profile=DEBUG_PROFILE)
        runner.artifacts = [("flower", debug_dll)]
        with self.assertRaisesRegex(
            flower_package.PackagingError, "非 release flower.dll"
        ):
            self.run_package(runner, "only-debug")

    def test_debug_path_is_rejected_even_if_profile_claims_release(self) -> None:
        debug_dir = (
            self.fixture.target_dir
            / flower_package.TARGET
            / "debug"
        )
        debug_dir.mkdir()
        debug_dll = debug_dir / "flower.dll"
        debug_dll.write_bytes(make_pe(is_dll=True))
        runner = self.fixture.runner()
        runner.artifacts = [
            ("flower", debug_dll),
            ("flower-settings", self.fixture.settings),
            ("flower-register", self.fixture.register),
        ]
        with self.assertRaisesRegex(
            flower_package.PackagingError, "不在 x86_64-pc-windows-msvc/release"
        ):
            self.run_package(runner, "debug-path")

    def test_missing_or_duplicate_artifact_is_rejected(self) -> None:
        runner = self.fixture.runner()
        runner.artifacts = [
            ("flower", self.fixture.dll),
            ("flower-settings", self.fixture.settings),
        ]
        with self.assertRaisesRegex(flower_package.PackagingError, "flower-register"):
            self.run_package(runner, "missing")

        runner = self.fixture.runner()
        runner.artifacts.append(("flower", self.fixture.dll))
        with self.assertRaisesRegex(flower_package.PackagingError, "歧义"):
            self.run_package(runner, "duplicate")

    def test_binary_validation_rejects_bad_arch_truncation_flags_and_exports(
        self,
    ) -> None:
        cases = []

        wrong_arch = bytearray(make_pe(is_dll=True))
        _u16(wrong_arch, 0x84, 0x014C)
        cases.append(("wrong arch", bytes(wrong_arch)))

        truncated = make_pe(is_dll=True)[:0x1B0]
        cases.append(("truncated", truncated))

        wrong_flag = bytearray(make_pe(is_dll=True))
        _u16(wrong_flag, 0x96, 0x0022)
        cases.append(("wrong DLL flag", bytes(wrong_flag)))

        cases.append(
            (
                "missing export",
                make_pe(is_dll=True, exports={"DllGetClassObject"}),
            )
        )

        wrong_rva = bytearray(make_pe(is_dll=True))
        _u32(wrong_rva, 0x234, 0x9000)
        cases.append(("function RVA outside image", bytes(wrong_rva)))

        unmapped_rva = bytearray(make_pe(is_dll=True))
        _u32(unmapped_rva, 0x234, 0x1800)
        cases.append(("function RVA not backed by a section", bytes(unmapped_rva)))

        wrong_ordinal = bytearray(make_pe(is_dll=True))
        _u16(wrong_ordinal, 0x244, 9)
        cases.append(("ordinal outside function table", bytes(wrong_ordinal)))

        forwarder = bytearray(make_pe(is_dll=True))
        _u32(forwarder, 0x234, 0x1000)
        cases.append(("forwarder", bytes(forwarder)))

        empty_name = bytearray(make_pe(is_dll=True))
        empty_name[0x248] = 0
        cases.append(("empty export name", bytes(empty_name)))

        overlapping_virtual = bytearray(make_pe(is_dll=True))
        section = 0x188
        _u16(overlapping_virtual, 0x86, 2)
        _u32(overlapping_virtual, section + 16, 0x200)
        second_section = section + 40
        _u32(overlapping_virtual, second_section + 8, 0x200)
        _u32(overlapping_virtual, second_section + 12, 0x1200)
        _u32(overlapping_virtual, second_section + 16, 0x100)
        _u32(overlapping_virtual, second_section + 20, 0x400)
        cases.append(("overlapping virtual sections", bytes(overlapping_virtual)))

        for index, (label, binary) in enumerate(cases):
            with self.subTest(label=label):
                fixture_path = self.fixture.dll
                fixture_path.write_bytes(binary)
                with self.assertRaises(flower_package.PackagingError):
                    self.run_package(self.fixture.runner(), f"bad-{index}")
                self.assertFalse((self.base / f"bad-{index}").exists())

    def test_missing_license_text_and_unknown_expression_are_rejected(self) -> None:
        unicode_dependency = next(
            package
            for package in self.fixture.dependencies
            if package["name"] == "unicode-ident"
        )
        unicode_license = (
            Path(unicode_dependency["manifest_path"]).parent / "LICENSE-UNICODE"
        )
        unicode_license.unlink()
        with self.assertRaisesRegex(flower_package.PackagingError, "许可证正文"):
            self.run_package(self.fixture.runner(), "missing-license")
        self.assertFalse((self.base / "missing-license").exists())

        unicode_dependency["license"] = "MIT OR Apache-2.0 OR LicenseRef-Unknown"
        with self.assertRaisesRegex(flower_package.PackagingError, "SPDX"):
            self.run_package(self.fixture.runner(), "unknown-license")

    def test_executable_marked_as_dll_is_rejected(self) -> None:
        self.fixture.settings.write_bytes(make_pe(is_dll=True))
        with self.assertRaisesRegex(flower_package.PackagingError, "不得带 DLL"):
            self.run_package(self.fixture.runner(), "exe-dll-flag")
        self.assertFalse((self.base / "exe-dll-flag").exists())

    def test_cargo_failures_stop_immediately_without_rustc_calls(self) -> None:
        runner = self.fixture.runner(build_exit=7)
        with self.assertRaisesRegex(flower_package.PackagingError, "退出码 7"):
            self.run_package(runner, "cargo-fail")
        self.assertEqual(
            [call[0][:2] for call in runner.calls],
            [["cargo", "metadata"], ["cargo", "build"]],
        )
        self.assertFalse((self.base / "cargo-fail").exists())

        runner = self.fixture.runner(metadata_exit=9)
        with self.assertRaisesRegex(flower_package.PackagingError, "退出码 9"):
            self.run_package(runner, "metadata-fail")
        self.assertEqual(len(runner.calls), 1)

    def test_existing_outputs_are_not_modified(self) -> None:
        output = self.base / "existing"
        output.mkdir()
        zip_path = output / "flower-0.1.0-windows-x64.zip"
        checksum_path = output / "flower-0.1.0-windows-x64.zip.sha256"
        zip_path.write_bytes(b"existing zip")
        checksum_path.write_bytes(b"existing checksum")
        runner = self.fixture.runner()
        with self.assertRaisesRegex(flower_package.PackagingError, "拒绝覆盖"):
            self.run_package(runner, "existing")
        self.assertEqual(zip_path.read_bytes(), b"existing zip")
        self.assertEqual(checksum_path.read_bytes(), b"existing checksum")
        self.assertEqual(len(runner.calls), 1)
        self.assertEqual(
            {path.name for path in output.iterdir()},
            {
                "flower-0.1.0-windows-x64.zip",
                "flower-0.1.0-windows-x64.zip.sha256",
            },
        )

    def test_unsafe_version_is_rejected_before_build(self) -> None:
        for index, version in enumerate(("../0.1.0", "1" * 129)):
            with self.subTest(version=version):
                self.fixture.metadata = self.fixture._metadata(version)
                runner = self.fixture.runner()
                with self.assertRaisesRegex(flower_package.PackagingError, "文件名"):
                    self.run_package(runner, f"unsafe-version-{index}")
                self.assertEqual(len(runner.calls), 1)
                self.assertFalse(any(self.base.rglob("*0.1.0.zip")))

    def test_failure_during_archive_write_removes_only_its_staging(self) -> None:
        output = self.base / "cleanup"
        output.mkdir()
        user_file = output / "keep.txt"
        user_file.write_text("keep", encoding="utf-8")
        with mock.patch.object(
            flower_package,
            "_write_zip",
            side_effect=OSError("simulated archive failure"),
        ):
            with self.assertRaises(OSError):
                self.run_package(self.fixture.runner(), "cleanup")
        self.assertEqual(user_file.read_text("utf-8"), "keep")
        self.assertEqual(
            {path.name for path in output.iterdir()},
            {"keep.txt"},
        )

    def test_second_publish_failure_rolls_back_first_product(self) -> None:
        output = self.base / "publish-failure"
        user_file = output / "keep.txt"
        output.mkdir()
        user_file.write_text("keep", encoding="utf-8")
        real_publish = flower_package._publish_no_overwrite
        publish_count = 0

        def fail_second(source, destination):
            nonlocal publish_count
            publish_count += 1
            if publish_count == 2:
                raise flower_package.PackagingError("模拟第二产物发布失败")
            return real_publish(source, destination)

        with mock.patch.object(
            flower_package, "_publish_no_overwrite", side_effect=fail_second
        ):
            with self.assertRaisesRegex(
                flower_package.PackagingError, "模拟第二产物发布失败"
            ):
                self.run_package(self.fixture.runner(), "publish-failure")
        self.assertEqual(user_file.read_text("utf-8"), "keep")
        self.assertEqual({path.name for path in output.iterdir()}, {"keep.txt"})

    def test_rollback_cleanup_failure_is_reported_with_original_error(self) -> None:
        output = self.base / "rollback-failure"
        output.mkdir()
        real_publish = flower_package._publish_no_overwrite
        publish_count = 0

        def fail_second(source, destination):
            nonlocal publish_count
            publish_count += 1
            if publish_count == 2:
                raise flower_package.PackagingError("模拟第二产物发布失败")
            return real_publish(source, destination)

        with mock.patch.object(
            flower_package, "_publish_no_overwrite", side_effect=fail_second
        ), mock.patch.object(
            Path,
            "unlink",
            side_effect=PermissionError("simulated unlink failure"),
        ):
            with self.assertRaises(flower_package.PackagingError) as raised:
                self.run_package(self.fixture.runner(), "rollback-failure")
        message = str(raised.exception)
        self.assertIn("模拟第二产物发布失败", message)
        self.assertIn("回滚失败", message)
        zip_path = output / "flower-0.1.0-windows-x64.zip"
        self.assertTrue(zip_path.is_file())
        self.assertFalse(
            (output / "flower-0.1.0-windows-x64.zip.sha256").exists()
        )
        self.assertFalse(any(output.glob(".flower-package-*")))

    def test_rollback_does_not_delete_externally_replaced_product(self) -> None:
        output = self.base / "external-replacement"
        output.mkdir()
        zip_path = output / "flower-0.1.0-windows-x64.zip"
        real_publish = flower_package._publish_no_overwrite
        publish_count = 0

        def replace_then_fail(source, destination):
            nonlocal publish_count
            publish_count += 1
            if publish_count == 2:
                zip_path.unlink()
                zip_path.write_bytes(b"external replacement")
                raise flower_package.PackagingError("模拟第二产物发布失败")
            return real_publish(source, destination)

        with mock.patch.object(
            flower_package, "_publish_no_overwrite", side_effect=replace_then_fail
        ):
            with self.assertRaises(flower_package.PackagingError) as raised:
                self.run_package(self.fixture.runner(), "external-replacement")
        self.assertIn("外部替换", str(raised.exception))
        self.assertEqual(zip_path.read_bytes(), b"external replacement")
        self.assertFalse(
            (output / "flower-0.1.0-windows-x64.zip.sha256").exists()
        )

    def test_missing_rust_docs_has_actionable_error(self) -> None:
        copyright_path = (
            self.fixture.sysroot
            / "share"
            / "doc"
            / "rust"
            / "COPYRIGHT-library.html"
        )
        copyright_path.unlink()
        with self.assertRaisesRegex(
            flower_package.PackagingError, "rust-docs"
        ):
            self.run_package(self.fixture.runner(), "no-rust-docs")

    def test_dangling_symlink_output_is_rejected(self) -> None:
        output = self.base / "dangling"
        output.mkdir()
        link = output / "flower-0.1.0-windows-x64.zip"
        try:
            link.symlink_to(output / "missing-target")
        except (OSError, NotImplementedError):
            self.skipTest("当前测试环境不能创建符号链接")
        with self.assertRaisesRegex(flower_package.PackagingError, "拒绝覆盖"):
            self.run_package(self.fixture.runner(), "dangling")
        self.assertTrue(flower_package._exists_even_dangling(link))


if __name__ == "__main__":
    unittest.main()
