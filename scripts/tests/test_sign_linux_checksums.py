"""Behaviour of scripts/sign_linux_checksums.sh with throwaway GPG keys.

Each test generates its own ed25519 key in a private GNUPGHOME, so the real
release key is never involved. GPG's agent socket must live on a short path
(macOS temp dirs are too long for a Unix socket), hence mkdtemp under /tmp.
"""

import hashlib
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "sign_linux_checksums.sh"
SUMS = "EmailOps-linux-SHA256SUMS"

pytestmark = pytest.mark.skipif(shutil.which("gpg") is None, reason="gpg not installed")


def _short_tmp() -> Path:
    return Path(tempfile.mkdtemp(dir="/tmp", prefix="sgt."))


def _make_key(uid: str) -> tuple[str, str]:
    """Return (armored private key, armored public key) for a fresh key."""
    home = _short_tmp()
    env = {**os.environ, "GNUPGHOME": str(home)}
    try:
        gpg = ["gpg", "--batch", "--pinentry-mode", "loopback", "--passphrase", ""]
        subprocess.run([*gpg, "--quick-gen-key", uid, "ed25519", "sign", "1d"], env=env, check=True, capture_output=True)
        private = subprocess.run([*gpg, "--armor", "--export-secret-keys", uid], env=env, check=True, capture_output=True, text=True).stdout
        public = subprocess.run(["gpg", "--batch", "--armor", "--export", uid], env=env, check=True, capture_output=True, text=True).stdout
        return private, public
    finally:
        subprocess.run(["gpgconf", "--kill", "gpg-agent"], env=env, capture_output=True)
        shutil.rmtree(home, ignore_errors=True)


@pytest.fixture(scope="module")
def release_key():
    return _make_key("Test Release <release@example.test>")


@pytest.fixture
def art(tmp_path):
    d = tmp_path / "release"
    d.mkdir()
    (d / "EmailOps-linux.deb").write_bytes(b"deb payload")
    (d / "EmailOps-linux.AppImage").write_bytes(b"appimage payload")
    (d / "notes.txt").write_text("not an installer")
    return d


def _run(target: Path, tmp_path: Path, private: str | None, public: str) -> subprocess.CompletedProcess:
    pub_file = tmp_path / "published.asc"
    pub_file.write_text(public)
    env = {**os.environ, "TMPDIR": "/tmp", "RELEASE_SIGNING_PUBLIC_KEY": str(pub_file)}
    env.pop("RELEASE_GPG_PRIVATE_KEY", None)
    if private is not None:
        env["RELEASE_GPG_PRIVATE_KEY"] = private
    return subprocess.run(["bash", str(SCRIPT), str(target)], env=env, capture_output=True, text=True, timeout=60)


def test_lists_the_linux_installers_with_their_sha256(art, tmp_path, release_key):
    result = _run(art, tmp_path, *release_key)

    assert result.returncode == 0, result.stderr
    lines = sorted((art / SUMS).read_text().splitlines())
    expected = sorted(
        f"{hashlib.sha256((art / name).read_bytes()).hexdigest()}  {name}"
        for name in ("EmailOps-linux.AppImage", "EmailOps-linux.deb")
    )
    assert lines == expected


def test_the_signature_verifies_against_the_published_key(art, tmp_path, release_key):
    _, public = release_key
    result = _run(art, tmp_path, *release_key)
    assert result.returncode == 0, result.stderr

    home = _short_tmp()
    env = {**os.environ, "GNUPGHOME": str(home)}
    try:
        subprocess.run(["gpg", "--batch", "--import"], input=public, env=env, check=True, capture_output=True, text=True)
        verify = subprocess.run(
            ["gpg", "--batch", "--verify", str(art / f"{SUMS}.asc"), str(art / SUMS)],
            env=env, capture_output=True, text=True,
        )
    finally:
        subprocess.run(["gpgconf", "--kill", "gpg-agent"], env=env, capture_output=True)
        shutil.rmtree(home, ignore_errors=True)
    assert verify.returncode == 0, verify.stderr


def test_a_key_that_is_not_the_published_one_fails(art, tmp_path, release_key):
    other_private, _ = _make_key("Someone Else <other@example.test>")
    _, published = release_key

    result = _run(art, tmp_path, other_private, published)

    assert result.returncode != 0
    assert "published" in result.stderr


def test_missing_private_key_fails_before_writing_anything(art, tmp_path, release_key):
    _, public = release_key

    result = _run(art, tmp_path, None, public)

    assert result.returncode != 0
    assert "RELEASE_GPG_PRIVATE_KEY" in result.stderr
    assert not (art / SUMS).exists()


def test_a_directory_without_installers_fails(tmp_path, release_key):
    empty = tmp_path / "release"
    empty.mkdir()
    (empty / "notes.txt").write_text("nothing to sign")

    result = _run(empty, tmp_path, *release_key)

    assert result.returncode != 0
    assert not (empty / SUMS).exists()
