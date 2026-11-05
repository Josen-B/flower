"""安装器构建边界测试；使用合成 PE 和注入 runner，不执行安装器。"""

from __future__ import annotations

import hashlib
import json
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

from scripts import installer
from scripts.test_package import PackageFixture, flower_package, make_pe


class InstallerSourceTests(unittest.TestCase):
    """Static integration guards, not a substitute for executing the wizard."""

    def test_non_elevated_copy_and_handle_only_rollback_contract(self):
        source = (installer.REPO / "src/platform/install_path.rs").read_text(encoding="utf-8-sig")
        copy = source.split("pub fn install_user_files", 1)[1].split("#[cfg(test)]", 1)[0]
        self.assertLess(copy.index("require_non_elevated()?"), copy.index("user_path_guards(path, true)"))
        self.assertIn("elevation.TokenIsElevated != 0", source)
        self.assertIn(".create_new(true)", copy)
        self.assertIn(".share_mode(0)", copy)
        self.assertIn("SetFileInformationByHandle", copy)
        self.assertNotIn("remove_file", copy)
        self.assertNotIn("remove_dir_all", copy)
        self.assertNotIn("fs::copy", copy)
        self.assertIn("user_directory_guard(directory)", source)

    def test_directory_selection_and_pre_copy_path_gate(self):
        source = (installer.REPO / "installer/flower.iss").read_text(encoding="utf-8-sig")
        self.assertIn("DisableDirPage=yes", source)
        self.assertIn(r"DefaultDirName={commonpf64}\Flower", source)
        self.assertIn(r"UninstallFilesDir={commonpf64}\Flower", source)
        self.assertNotIn("{app}", source)
        self.assertIn("CreateInputDirPage", source)
        self.assertIn("AllowUNCPath=no", source)
        self.assertIn("Flags: dontcopy", source)
        prepare = source.split("function PrepareToInstall", 1)[1].split("procedure CurStepChanged", 1)[0]
        self.assertIn(r"SafeInstallPath(ExpandConstant('{commonpf64}\Flower'))", prepare)
        checker = source.split("function SafeInstallPath", 1)[1].split("function NextButtonClick", 1)[0]
        self.assertLess(checker.index("GetSHA256OfFile"), checker.index("if Exec("))
        self.assertIn("check-install-path --path", checker)
        page = source.split("function NextButtonClick", 1)[1].split("function PrepareInstallPath", 1)[0]
        self.assertIn("check-user-install-path --path", page)
        self.assertNotIn("SafeInstallPath(WizardDirValue)", page)
        self.assertNotIn("prepare-install-path", page)
        self.assertNotIn("PrepareInstallPath(", page)
        for gate in ["HasFlowerRegistration", "DirExists(PayloadDir)", "HasNativeRuntime"]:
            self.assertLess(prepare.index(gate), prepare.index("PrepareInstallPath("))
        self.assertLess(prepare.index("PrepareInstallPath("), prepare.index("SafeInstallPath("))
        creator = source.split("function PrepareInstallPath", 1)[1].split("function HasSubkeys", 1)[0]
        self.assertLess(creator.index("GetSHA256OfFile"), creator.index("if Exec("))
        self.assertIn("prepare-install-path --path", creator)

    def test_post_copy_and_uninstall_validate_before_executing_installed_tool(self):
        source = (installer.REPO / "installer/flower.iss").read_text(encoding="utf-8-sig")
        post = source.split("procedure CurStepChanged", 1)[1].split("procedure CurPageChanged", 1)[0]
        self.assertLess(post.index("SafeInstallPath"), post.index("if Exec(PayloadDir"))
        self.assertNotIn("PrepareInstallPath(", post)
        self.assertIn("ExecAsOriginalUser", post)
        self.assertIn("install-user-files --path", post)
        self.assertLess(post.index("TrustedPayload"), post.index("ExecAsOriginalUser"))
        self.assertIn("UserFilesFailed := ExitCode <> 0", post)
        self.assertIn("if UserFilesFailed then Result := 16", source)
        uninstall = source.split("function InitializeUninstall", 1)[1]
        self.assertLess(uninstall.index("FileCopy("), uninstall.index("SafeInstallPath"))
        self.assertLess(uninstall.index("SafeInstallPath"), uninstall.index("if not Exec(PayloadDir"))
        self.assertNotIn("PrepareInstallPath(", uninstall)


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        fixture = PackageFixture(self.base)
        self.archive = flower_package.build_package(
            repo_root=fixture.repo, output_dir=self.base / "zip",
            host_platform="win32", runner=fixture.runner(),
        ).zip_path
        self.compiler = self.base / "compiler with spaces" / "ISCC.exe"
        self.compiler.parent.mkdir()
        self.compiler.write_bytes(b"test runner only")
        (self.compiler.parent / "license.txt").write_bytes(
            b"Inno Setup License\nCopyright Jordan Russell\nTest fixture, not a distributable license.\n"
        )
        self.output = self.base / "installer output"
        self.calls = 0

    def runner(self, args, cwd):
        self.calls += 1
        self.assertEqual(cwd, installer.REPO)
        options = {
            arg[2:].split("=", 1)[0]: arg.split("=", 1)[1]
            for arg in args if arg.startswith("/D") and "=" in arg
        }
        payload = Path(options["PackageDir"])
        self.assertEqual(options["FileVersion"], "0.1.0.0")
        include = Path(options["FilesInclude"]).read_text(encoding="utf-8-sig")
        self.assertEqual(len(include.splitlines()), len(list(payload.rglob("*"))) -
                         len([p for p in payload.rglob("*") if p.is_dir()]))
        self.assertNotIn("recursesubdirs", include)
        self.assertIn(r'DestDir: "{commonpf64}\Flower\versions\{#BuildId}', include)
        self.assertNotIn("{app}", include)
        self.assertEqual(
            options["ExpectedDllHash"],
            hashlib.sha256((payload / "flower.dll").read_bytes()).hexdigest(),
        )
        output = Path(options["OutputDir"]) / f"flower-{options['AppVersion']}-windows-x64-setup.exe"
        output.write_bytes(make_pe(is_dll=False))
        return subprocess.CompletedProcess(args, 0, "", "")

    def build(self, **options):
        return installer.build_installer(
            self.archive, self.output, compiler=self.compiler,
            runner=options.pop("runner", self.runner), host="win32", **options,
        )

    def rewrite(self, mutate, *, rehash_payload=False):
        with zipfile.ZipFile(self.archive) as source:
            contents = {info.filename: source.read(info) for info in source.infolist()}
        mutate(contents)
        if rehash_payload:
            sums = next(name for name in contents if name.endswith("/SHA256SUMS"))
            top = sums.rsplit("/", 1)[0]
            contents[sums] = "".join(
                f"{hashlib.sha256(data).hexdigest()}  {name[len(top)+1:]}\n"
                for name, data in sorted(contents.items()) if name != sums
            ).encode()
        with zipfile.ZipFile(self.archive, "w") as target:
            for name, data in contents.items():
                target.writestr(name, data)
        self.refresh_external_hash()

    def refresh_external_hash(self):
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        Path(f"{self.archive}.sha256").write_text(f"{digest}  {self.archive.name}\n")

    def test_valid_package_and_build_hashes_manifest_and_cleanup(self):
        verified = installer.verify_package(self.archive)
        self.assertEqual(verified.version, "0.1.0")
        result = self.build()
        self.assertEqual(self.calls, 1)
        digest = hashlib.sha256(result.executable.read_bytes()).hexdigest()
        self.assertEqual(result.checksum.read_text().split()[0], digest)
        record = json.loads(result.manifest.read_text())
        self.assertEqual(record["source_zip_sha256"], verified.digest)
        self.assertFalse(record["installation_runtime_verified"])
        self.assertFalse(record["windows_desktop_verified"])
        self.assertNotIn(str(self.base), result.manifest.read_text())
        self.assertEqual(len(list(self.output.iterdir())), 3)

    def test_external_checksum_mismatch_stops_before_compiler(self):
        Path(f"{self.archive}.sha256").write_text("0" * 64 + f"  {self.archive.name}\n")
        with self.assertRaisesRegex(installer.InstallerError, "外部"):
            self.build()
        self.assertEqual(self.calls, 0)

    def test_payload_tamper_is_rejected(self):
        self.rewrite(lambda data: data.update({next(k for k in data if k.endswith("/README.md")): b"changed"}))
        with self.assertRaisesRegex(installer.InstallerError, "哈希"):
            self.build()

    def test_unsafe_paths_multiple_roots_and_extra_payload_are_rejected(self):
        cases = [
            "../outside", "/absolute/file", "root/C:/bad", "root/a\\b",
            "root/CON", "root/CON .txt", "root/COM¹.txt",
            "root/a.", "root/file:stream", "root/{app}",
            "another-root/README.md",
            "flower-0.1.0-windows-x64/unexpected.exe",
            "flower-0.1.0-windows-x64/licenses/evil.exe",
        ]
        original = self.archive.read_bytes()
        for name in cases:
            with self.subTest(name=name):
                self.archive.write_bytes(original)
                self.rewrite(lambda data: data.update({name: b"unsafe"}))
                with self.assertRaises(installer.InstallerError):
                    installer.verify_package(self.archive)

    def test_case_insensitive_collision_is_rejected(self):
        self.rewrite(lambda data: data.update({
            "flower-0.1.0-windows-x64/readme.MD": b"duplicate",
        }))
        with self.assertRaises(installer.InstallerError):
            self.build()

    def test_zip_symlink_is_rejected(self):
        with zipfile.ZipFile(self.archive, "a") as target:
            info = zipfile.ZipInfo("flower-0.1.0-windows-x64/licenses/link")
            info.create_system = 3
            info.external_attr = 0o120777 << 16
            target.writestr(info, "outside")
        self.refresh_external_hash()
        with self.assertRaisesRegex(installer.InstallerError, "非普通文件"):
            self.build()

    def test_missing_payload_is_rejected(self):
        self.rewrite(lambda data: data.pop(next(k for k in data if k.endswith("/flower.dll"))))
        with self.assertRaises(installer.InstallerError):
            self.build()

    def test_manifest_metadata_mismatch_is_rejected_even_with_rehashed_payload(self):
        def mutate(data):
            name = f"{self.archive.stem}/manifest.json"
            value = json.loads(data[name])
            value["binaries"][0]["size"] = 1
            data[name] = json.dumps(value).encode()
        self.rewrite(mutate, rehash_payload=True)
        with self.assertRaisesRegex(installer.InstallerError, "元数据"):
            self.build()

    def test_manifest_must_be_a_json_object(self):
        def mutate(data):
            name = f"{self.archive.stem}/manifest.json"
            data[name] = b"[]"
        self.rewrite(mutate, rehash_payload=True)
        with self.assertRaisesRegex(installer.InstallerError, "JSON"):
            self.build()

    def test_version_must_fit_windows_resource_fields(self):
        def mutate(data):
            name = f"{self.archive.stem}/manifest.json"
            value = json.loads(data[name])
            value["version"] = "70000.1.0"
            data[name] = json.dumps(value).encode()
        self.rewrite(mutate, rehash_payload=True)
        with self.assertRaisesRegex(installer.InstallerError, "版本资源"):
            self.build()

    def test_existing_output_is_preserved(self):
        self.output.mkdir()
        name = self.output / "flower-0.1.0-windows-x64-setup.exe"
        name.write_bytes(b"user-owned")
        with self.assertRaisesRegex(installer.InstallerError, "拒绝覆盖"):
            self.build()
        self.assertEqual(name.read_bytes(), b"user-owned")
        self.assertEqual(self.calls, 0)

    def test_compiler_failure_has_no_products_and_cleans_staging(self):
        def fail(args, cwd):
            return subprocess.CompletedProcess(args, 1, "", "intentional failure")
        with self.assertRaisesRegex(installer.InstallerError, "intentional failure"):
            self.build(runner=fail)
        self.assertEqual(list(self.output.iterdir()), [])

    def test_invalid_compiler_output_is_rejected(self):
        def invalid(args, cwd):
            self.runner(args, cwd)
            output = next(Path(arg.split("=", 1)[1]) for arg in args if arg.startswith("/DOutputDir="))
            next(output.glob("*.exe")).write_bytes(b"not a PE")
            return subprocess.CompletedProcess(args, 0, "", "")
        with self.assertRaisesRegex(installer.InstallerError, "PE"):
            self.build(runner=invalid)
        self.assertEqual(list(self.output.iterdir()), [])

    def test_publish_failure_rolls_back_only_owned_products(self):
        publish = installer.pack._publish_no_overwrite
        calls = 0
        def fail_second(source, destination):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise installer.InstallerError("injected publish failure")
            return publish(source, destination)
        with mock.patch.object(installer.pack, "_publish_no_overwrite", side_effect=fail_second):
            with self.assertRaisesRegex(installer.InstallerError, "injected"):
                self.build()
        self.assertEqual(list(self.output.iterdir()), [])

    def test_non_windows_internal_entry_is_refused(self):
        with self.assertRaisesRegex(installer.InstallerError, "Windows"):
            installer.build_installer(self.archive, self.output, host="linux")

    def test_missing_explicit_compiler_is_not_silently_replaced(self):
        with self.assertRaisesRegex(installer.InstallerError, "编译器"):
            installer.find_compiler(self.base / "missing.exe")

    def test_unrecognized_engine_license_is_rejected(self):
        (self.compiler.parent / "license.txt").write_bytes(b"not a compiler license")
        with self.assertRaisesRegex(installer.InstallerError, "许可证"):
            self.build()
        self.assertEqual(self.calls, 0)

    def test_help_succeeds_without_building(self):
        with mock.patch("sys.stdout"):
            with self.assertRaises(SystemExit) as error:
                installer.main(["--help"])
        self.assertEqual(error.exception.code, 0)


if __name__ == "__main__":
    unittest.main()
