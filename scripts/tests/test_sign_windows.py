"""Behaviour of scripts/sign_windows.sh, run against stub signing tools.

The real tools need a Certum login, so each test puts fake `ssign` and
`osslsigncode` executables first on PATH. The fakes log every call and mark
the files they sign, which lets the tests check what was signed, with which
tool, and that a failed verification fails the script.
"""

import os
import stat
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "sign_windows.sh"
MARK = b"\nSIGNED"

FAKE_SSIGN = """#!/usr/bin/env bash
echo "ssign $*" >> "$CALL_LOG"
for arg in "$@"; do
  case "$arg" in
    *.exe|*.dll|*.EXE|*.DLL) printf '\\nSIGNED' >> "$arg" ;;
  esac
done
"""

FAKE_OSSLSIGNCODE = """#!/usr/bin/env bash
echo "osslsigncode $*" >> "$CALL_LOG"
cmd="$1"; shift
in=""; out=""
while [ $# -gt 0 ]; do
  case "$1" in
    -in) in="$2"; shift ;;
    -out) out="$2"; shift ;;
  esac
  shift
done
if [ "$cmd" = sign ]; then
  cp "$in" "$out" && printf '\\nSIGNED' >> "$out"
elif [ "$cmd" = verify ]; then
  case "$in" in *"${FAIL_VERIFY:-<none>}"*) exit 1 ;; esac
  tail -c 7 "$in" | grep -q SIGNED || exit 1
fi
"""


def _install(bin_dir: Path, name: str, body: str) -> None:
    path = bin_dir / name
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


@pytest.fixture
def env(tmp_path):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _install(bin_dir, "ssign", FAKE_SSIGN)
    _install(bin_dir, "osslsigncode", FAKE_OSSLSIGNCODE)
    module = tmp_path / "libssign_pkcs11.so"
    module.write_bytes(b"module")
    intermediate = tmp_path / "intermediate.pem"
    intermediate.write_text("-----BEGIN CERTIFICATE-----\n")
    return {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "CALL_LOG": str(tmp_path / "calls.log"),
        "CERTUM_EMAIL": "signer@example.test",
        "CERTUM_OTP": "JBSWY3DPEHPK3PXP",
        "SSIGN_PKCS11_MODULE": str(module),
        "CERTUM_INTERMEDIATE_PEM": str(intermediate),
    }


def _run(target: Path, env: dict) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(SCRIPT), str(target)],
        env=env,
        capture_output=True,
        text=True,
    )


def _calls(env: dict) -> list[str]:
    log = Path(env["CALL_LOG"])
    return log.read_text().splitlines() if log.exists() else []


def _write(path: Path, data: bytes = b"MZ payload") -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return path


def test_signs_every_pe_file_in_one_ssign_login(tmp_path, env):
    exe = _write(tmp_path / "art" / "emailops.exe")
    dll = _write(tmp_path / "art" / "backends" / "ggml-base.dll")

    result = _run(tmp_path / "art", env)

    assert result.returncode == 0, result.stderr
    ssign_calls = [c for c in _calls(env) if c.startswith("ssign ")]
    assert len(ssign_calls) == 1
    assert str(exe) in ssign_calls[0] and str(dll) in ssign_calls[0]
    assert exe.read_bytes().endswith(MARK)
    assert dll.read_bytes().endswith(MARK)


def test_signs_msi_through_the_pkcs11_module_in_place(tmp_path, env):
    msi = _write(tmp_path / "art" / "EmailOps-windows.msi", b"OLE payload")

    result = _run(tmp_path / "art", env)

    assert result.returncode == 0, result.stderr
    sign_calls = [c for c in _calls(env) if c.startswith("osslsigncode sign")]
    assert len(sign_calls) == 1
    assert f"-pkcs11module {env['SSIGN_PKCS11_MODULE']}" in sign_calls[0]
    assert f"-ac {env['CERTUM_INTERMEDIATE_PEM']}" in sign_calls[0]
    assert msi.read_bytes().endswith(MARK)
    assert not any(c.startswith("ssign ") for c in _calls(env))


def test_pe_files_are_signed_before_the_msi_reuses_the_session(tmp_path, env):
    _write(tmp_path / "art" / "EmailOps-windows-setup.exe")
    _write(tmp_path / "art" / "EmailOps-windows.msi", b"OLE payload")

    result = _run(tmp_path / "art", env)

    assert result.returncode == 0, result.stderr
    calls = _calls(env)
    first_ssign = next(i for i, c in enumerate(calls) if c.startswith("ssign "))
    first_msi = next(i for i, c in enumerate(calls) if c.startswith("osslsigncode sign"))
    assert first_ssign < first_msi


def test_verifies_every_signed_file(tmp_path, env):
    exe = _write(tmp_path / "art" / "emailops.exe")
    msi = _write(tmp_path / "art" / "EmailOps-windows.msi", b"OLE payload")

    result = _run(tmp_path / "art", env)

    assert result.returncode == 0, result.stderr
    verify_calls = [c for c in _calls(env) if c.startswith("osslsigncode verify")]
    assert any(str(exe) in c for c in verify_calls)
    assert any(str(msi) in c for c in verify_calls)


def test_a_failed_verification_fails_the_run_and_names_the_file(tmp_path, env):
    _write(tmp_path / "art" / "emailops.exe")
    _write(tmp_path / "art" / "backends" / "ggml-vulkan.dll")
    env["FAIL_VERIFY"] = "ggml-vulkan.dll"

    result = _run(tmp_path / "art", env)

    assert result.returncode != 0
    assert "ggml-vulkan.dll" in result.stderr


def test_an_empty_directory_is_an_error(tmp_path, env):
    (tmp_path / "art").mkdir()

    result = _run(tmp_path / "art", env)

    assert result.returncode != 0
    assert _calls(env) == []


@pytest.mark.parametrize("missing", ["CERTUM_EMAIL", "CERTUM_OTP"])
def test_missing_credentials_fail_before_any_signing(tmp_path, env, missing):
    _write(tmp_path / "art" / "emailops.exe")
    del env[missing]

    result = _run(tmp_path / "art", env)

    assert result.returncode != 0
    assert missing in result.stderr
    assert _calls(env) == []


def test_an_msi_without_the_pkcs11_module_fails_before_any_signing(tmp_path, env):
    _write(tmp_path / "art" / "emailops.exe")
    _write(tmp_path / "art" / "EmailOps-windows.msi", b"OLE payload")
    del env["SSIGN_PKCS11_MODULE"]

    result = _run(tmp_path / "art", env)

    assert result.returncode != 0
    assert "SSIGN_PKCS11_MODULE" in result.stderr
    assert _calls(env) == []
