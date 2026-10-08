from __future__ import annotations

import hashlib
import io
import lzma
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tarfile
import tempfile
import unittest


PROVISIONER = (
    Path(__file__).resolve().parents[3] / "engine" / "firmware" / "S90spotui-provision"
)
RUNTIME_FILES = {
    "spotui-ui-poc": b"test ui",
    "spotui_daemon": b"test daemon",
    "ld-musl-mipsel-sf.so.1": b"test loader",
    "start_spotui.sh": b"#!/bin/sh\n",
    "start_spotui.real.sh": b"#!/bin/sh\n",
    "return_to_hiby.sh": b"#!/bin/sh\n",
}


class ProvisionRetryTests(unittest.TestCase):
    def make_environment(
        self, root: Path, failed_attempts: int
    ) -> tuple[dict[str, str], Path, Path, Path]:
        installer = root / "installer"
        sd_root = root / "sd"
        destination = root / "destination"
        fake_bin = root / "bin"
        for directory in (installer, sd_root, destination, fake_bin):
            directory.mkdir()

        version = b"spotui-test\n"
        manifest = "".join(
            f"{hashlib.sha256(content).hexdigest()}  {name}\n"
            for name, content in RUNTIME_FILES.items()
        ).encode("ascii")
        (installer / "VERSION").write_bytes(version)
        (installer / "SHA256SUMS").write_bytes(manifest)
        (installer / "PAYLOAD_BYTES").write_text(
            f"{sum(len(content) for content in RUNTIME_FILES.values())}\n",
            encoding="ascii",
        )

        archive_buffer = io.BytesIO()
        with tarfile.open(fileobj=archive_buffer, mode="w") as archive:
            archive_values = {
                **RUNTIME_FILES,
                "VERSION": version,
                "SHA256SUMS": manifest,
            }
            for name, content in archive_values.items():
                info = tarfile.TarInfo(name)
                info.size = len(content)
                info.mode = 0o755 if name in RUNTIME_FILES else 0o644
                archive.addfile(info, io.BytesIO(content))
        compressed = lzma.compress(
            archive_buffer.getvalue(),
            format=lzma.FORMAT_XZ,
            check=lzma.CHECK_CRC32,
            preset=4,
        )
        archive_path = sd_root / "spotui-runtime.tar.xz"
        archive_path.write_bytes(compressed)
        (installer / "spotui-runtime.tar.xz.sha256").write_text(
            f"{hashlib.sha256(compressed).hexdigest()}  spotui-runtime.tar.xz\n",
            encoding="ascii",
        )

        counter = root / "xz-attempts"
        real_xz = shutil.which("xz")
        assert real_xz is not None
        fake_xz = fake_bin / "xz"
        fake_xz.write_text(
            "#!/bin/sh\n"
            f"counter={shlex.quote(str(counter))}\n"
            "count=0\n"
            'if [ -f "$counter" ]; then read -r count < "$counter"; fi\n'
            "count=$((count + 1))\n"
            'echo "$count" > "$counter"\n'
            f"if [ \"$count\" -le {failed_attempts} ]; then exit 1; fi\n"
            f"exec {shlex.quote(real_xz)} \"$@\"\n",
            encoding="utf-8",
        )
        fake_xz.chmod(0o755)

        log = root / "provision.log"
        lock = root / "provision.lock"
        environment = os.environ.copy()
        environment.update(
            {
                "PATH": f"{fake_bin}:{environment['PATH']}",
                "SPOTUI_INSTALLER_DIR": str(installer),
                "SPOTUI_SD_ROOT": str(sd_root),
                "SPOTUI_DEST": str(destination),
                "SPOTUI_PROVISION_LOG": str(log),
                "SPOTUI_PROVISION_LOCK": str(lock),
                "SPOTUI_PROVISION_WAIT_SECONDS": "0",
                "SPOTUI_PROVISION_EXTRACT_INITIAL_DELAY_SECONDS": "0",
                "SPOTUI_PROVISION_EXTRACT_ATTEMPTS": "2",
                "SPOTUI_PROVISION_EXTRACT_RETRY_SECONDS": "0",
            }
        )
        return environment, destination, log, counter

    def run_provisioner(self, environment: dict[str, str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/sh", str(PROVISIONER), "worker"],
            check=False,
            capture_output=True,
            text=True,
            env=environment,
        )

    def test_second_extraction_attempt_succeeds(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, counter = self.make_environment(root, 1)

            result = self.run_provisioner(environment)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(counter.read_text(encoding="ascii").strip(), "2")
            for name, content in RUNTIME_FILES.items():
                self.assertEqual((destination / name).read_bytes(), content)
            log_text = log.read_text(encoding="utf-8")
            self.assertIn("runtime extraction attempt 1 failed", log_text)
            self.assertIn("installed runtime spotui-test", log_text)
            self.assertFalse((destination / ".spotui-install-stage").exists())
            self.assertFalse((root / "provision.lock").exists())

    def test_exhausted_attempts_leave_no_partial_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, counter = self.make_environment(root, 2)

            result = self.run_provisioner(environment)

            self.assertEqual(result.returncode, 1)
            self.assertEqual(counter.read_text(encoding="ascii").strip(), "2")
            for name in RUNTIME_FILES:
                self.assertFalse((destination / name).exists())
            self.assertIn(
                "could not extract runtime archive after 2 attempts",
                log.read_text(encoding="utf-8"),
            )
            self.assertFalse((destination / ".spotui-install-stage").exists())
            self.assertFalse((root / "provision.lock").exists())


    def write_managed_runtime(
        self,
        destination: Path,
        *,
        tamper: bool = False,
    ) -> dict[str, bytes]:
        old_files = {
            name: b"old-runtime:" + name.encode("ascii") + b"\n"
            for name in RUNTIME_FILES
        }

        for name, content in old_files.items():
            (destination / name).write_bytes(content)

        manifest = "".join(
            f"{hashlib.sha256(content).hexdigest()}  {name}\n"
            for name, content in old_files.items()
        ).encode("ascii")

        (destination / "spotui-runtime-version").write_bytes(
            b"spotui-old-test\n"
        )
        (destination / "spotui-runtime-SHA256SUMS").write_bytes(manifest)

        if tamper:
            (destination / "start_spotui.real.sh").write_bytes(
                b"tampered-runtime\n"
            )

        return old_files

    def test_exact_runtime_without_metadata_backfills_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, _counter = self.make_environment(
                root, 0
            )

            for name, content in RUNTIME_FILES.items():
                (destination / name).write_bytes(content)

            result = self.run_provisioner(environment)

            self.assertEqual(result.returncode, 0, result.stderr)

            installer = Path(environment["SPOTUI_INSTALLER_DIR"])
            self.assertEqual(
                (destination / "spotui-runtime-version").read_bytes(),
                (installer / "VERSION").read_bytes(),
            )
            self.assertEqual(
                (destination / "spotui-runtime-SHA256SUMS").read_bytes(),
                (installer / "SHA256SUMS").read_bytes(),
            )
            self.assertIn(
                "runtime already matches packaged version",
                log.read_text(encoding="utf-8"),
            )

    def test_managed_older_runtime_is_upgraded(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, _counter = self.make_environment(
                root, 0
            )

            self.write_managed_runtime(destination)

            result = self.run_provisioner(environment)

            self.assertEqual(result.returncode, 0, result.stderr)

            for name, content in RUNTIME_FILES.items():
                self.assertEqual(
                    (destination / name).read_bytes(),
                    content,
                )

            installer = Path(environment["SPOTUI_INSTALLER_DIR"])
            self.assertEqual(
                (destination / "spotui-runtime-version").read_bytes(),
                (installer / "VERSION").read_bytes(),
            )
            self.assertEqual(
                (destination / "spotui-runtime-SHA256SUMS").read_bytes(),
                (installer / "SHA256SUMS").read_bytes(),
            )
            self.assertIn(
                "installed runtime spotui-test",
                log.read_text(encoding="utf-8"),
            )

    def test_tampered_managed_runtime_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, _counter = self.make_environment(
                root, 0
            )

            self.write_managed_runtime(destination, tamper=True)

            before = {
                path.name: path.read_bytes()
                for path in destination.iterdir()
                if path.is_file()
            }

            result = self.run_provisioner(environment)

            self.assertNotEqual(result.returncode, 0)

            after = {
                path.name: path.read_bytes()
                for path in destination.iterdir()
                if path.is_file()
            }

            self.assertEqual(after, before)
            self.assertIn(
                "refusing automatic overwrite",
                log.read_text(encoding="utf-8"),
            )

    def test_unmanaged_existing_runtime_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, _counter = self.make_environment(
                root, 0
            )

            existing = destination / "start_spotui.real.sh"
            existing.write_bytes(b"local-custom-runtime\n")

            result = self.run_provisioner(environment)

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(
                existing.read_bytes(),
                b"local-custom-runtime\n",
            )
            self.assertIn(
                "refusing automatic overwrite",
                log.read_text(encoding="utf-8"),
            )

    def test_failed_upgrade_extraction_preserves_old_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, counter = self.make_environment(
                root, 2
            )

            self.write_managed_runtime(destination)

            before = {
                path.name: path.read_bytes()
                for path in destination.iterdir()
                if path.is_file()
            }

            result = self.run_provisioner(environment)

            self.assertNotEqual(result.returncode, 0)

            after = {
                path.name: path.read_bytes()
                for path in destination.iterdir()
                if path.is_file()
            }

            self.assertEqual(after, before)
            self.assertTrue(
                counter.exists(),
                "managed upgrade should reach archive extraction",
            )

            if counter.exists():
                self.assertEqual(
                    counter.read_text(encoding="ascii").strip(),
                    "2",
                )

            self.assertIn(
                "could not extract runtime archive after 2 attempts",
                log.read_text(encoding="utf-8"),
            )
            self.assertFalse(
                (destination / ".spotui-install-stage").exists()
            )


    def test_failed_activation_restores_managed_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, _counter = self.make_environment(
                root, 0
            )

            self.write_managed_runtime(destination)

            before = {
                path.name: path.read_bytes()
                for path in destination.iterdir()
                if path.is_file()
            }

            fake_bin = Path(environment["PATH"].split(":", 1)[0])
            real_mv = shutil.which("mv")
            self.assertIsNotNone(real_mv)

            activation_counter = root / "activation-mv-attempts"
            stage = destination / ".spotui-install-stage"

            fake_mv = fake_bin / "mv"
            fake_mv.write_text(
                "#!/bin/sh\n"
                f"stage={shlex.quote(str(stage))}\n"
                f"counter={shlex.quote(str(activation_counter))}\n"
                "case \"$1\" in\n"
                "    \"$stage\"/*)\n"
                "        count=0\n"
                "        if [ -f \"$counter\" ]; then\n"
                "            read -r count < \"$counter\"\n"
                "        fi\n"
                "        count=$((count + 1))\n"
                "        echo \"$count\" > \"$counter\"\n"
                "        if [ \"$count\" -eq 3 ]; then\n"
                "            exit 1\n"
                "        fi\n"
                "        ;;\n"
                "esac\n"
                f"exec {shlex.quote(real_mv)} \"$@\"\n",
                encoding="utf-8",
            )
            fake_mv.chmod(0o755)

            result = self.run_provisioner(environment)

            self.assertNotEqual(result.returncode, 0)

            after = {
                path.name: path.read_bytes()
                for path in destination.iterdir()
                if path.is_file()
            }

            self.assertEqual(
                after,
                before,
                "failed activation must restore the complete old runtime",
            )

            self.assertTrue(activation_counter.exists())
            self.assertEqual(
                activation_counter.read_text(encoding="ascii").strip(),
                "3",
            )

            self.assertFalse(
                (destination / ".spotui-install-stage").exists()
            )
            self.assertFalse(
                (destination / ".spotui-install-backup").exists()
            )

            self.assertIn(
                "could not activate runtime file",
                log.read_text(encoding="utf-8"),
            )


    def test_failed_permission_step_restores_managed_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment, destination, log, _counter = self.make_environment(
                root, 0
            )

            self.write_managed_runtime(destination)

            before_bytes = {
                path.name: path.read_bytes()
                for path in destination.iterdir()
                if path.is_file()
            }
            before_modes = {
                path.name: path.stat().st_mode & 0o7777
                for path in destination.iterdir()
                if path.is_file()
            }

            fake_bin = Path(environment["PATH"].split(":", 1)[0])
            real_chmod = shutil.which("chmod")
            self.assertIsNotNone(real_chmod)

            fake_chmod = fake_bin / "chmod"
            fake_chmod.write_text(
                "#!/bin/sh\n"
                f"dest={shlex.quote(str(destination))}\n"
                f"real={shlex.quote(real_chmod)}\n"
                "case \" $* \" in\n"
                "    *\" $dest/spotui-ui-poc \"*)\n"
                "        \"$real\" \"$@\" || exit $?\n"
                "        exit 1\n"
                "        ;;\n"
                "esac\n"
                "exec \"$real\" \"$@\"\n",
                encoding="utf-8",
            )
            fake_chmod.chmod(0o755)

            result = self.run_provisioner(environment)

            self.assertNotEqual(result.returncode, 0)

            after_bytes = {
                path.name: path.read_bytes()
                for path in destination.iterdir()
                if path.is_file()
            }
            after_modes = {
                path.name: path.stat().st_mode & 0o7777
                for path in destination.iterdir()
                if path.is_file()
            }

            self.assertEqual(after_bytes, before_bytes)
            self.assertEqual(after_modes, before_modes)

            self.assertIn(
                "could not enforce installed runtime permissions",
                log.read_text(encoding="utf-8"),
            )
            self.assertIn(
                "restored previous managed runtime after activation failure",
                log.read_text(encoding="utf-8"),
            )

            self.assertFalse(
                (destination / ".spotui-install-stage").exists()
            )
            self.assertFalse(
                (destination / ".spotui-install-backup").exists()
            )


if __name__ == "__main__":
    unittest.main()
