"""scripts/build_target.sh: link and clean of src-tauri/target on the Build volume.

A worktree may point its target at another checkout's warm build dir on purpose
(a fresh one means a full llama.cpp rebuild). `link` must keep that link, and
`clean` must never empty a build dir that belongs to another checkout.
"""

import os
import subprocess
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "build_target.sh"


def _checkout(tmp_path: Path, name: str = "wt") -> Path:
    root = tmp_path / name
    (root / "src-tauri").mkdir(parents=True)
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    return root


def _run(root: Path, volume: Path, action: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(SCRIPT), action],
        cwd=root,
        env={**os.environ, "BUILD_VOLUME": str(volume)},
        capture_output=True,
        text=True,
        timeout=30,
    )


def _build_dir(volume: Path, checkout: str) -> Path:
    path = volume / "emailops" / checkout / "target"
    path.mkdir(parents=True)
    (path / "artifact").write_text("warm build")
    return path


def test_clean_refuses_another_checkouts_target(tmp_path):
    root = _checkout(tmp_path)
    volume = tmp_path / "vol"
    main_target = _build_dir(volume, "main")
    (root / "src-tauri" / "target").symlink_to(main_target)

    result = _run(root, volume, "clean")

    assert result.returncode == 1
    assert (main_target / "artifact").exists()


def test_clean_empties_own_target(tmp_path):
    root = _checkout(tmp_path)
    volume = tmp_path / "vol"
    own_target = _build_dir(volume, "wt")
    (root / "src-tauri" / "target").symlink_to(own_target)

    result = _run(root, volume, "clean")

    assert result.returncode == 0
    assert own_target.is_dir()
    assert list(own_target.iterdir()) == []


def test_link_keeps_link_to_another_checkouts_target(tmp_path):
    root = _checkout(tmp_path)
    volume = tmp_path / "vol"
    main_target = _build_dir(volume, "main")
    link = root / "src-tauri" / "target"
    link.symlink_to(main_target)

    result = _run(root, volume, "link")

    assert result.returncode == 0
    assert os.readlink(link) == str(main_target)


def test_link_replaces_dangling_link(tmp_path):
    root = _checkout(tmp_path)
    volume = tmp_path / "vol"
    volume.mkdir()
    link = root / "src-tauri" / "target"
    link.symlink_to(volume / "emailops" / "gone" / "target")

    result = _run(root, volume, "link")

    assert result.returncode == 0
    assert os.readlink(link) == str(volume / "emailops" / "wt" / "target")


def test_link_creates_own_target(tmp_path):
    root = _checkout(tmp_path)
    volume = tmp_path / "vol"
    volume.mkdir()

    result = _run(root, volume, "link")

    assert result.returncode == 0
    own_target = volume / "emailops" / "wt" / "target"
    assert os.readlink(root / "src-tauri" / "target") == str(own_target)
    assert own_target.is_dir()
