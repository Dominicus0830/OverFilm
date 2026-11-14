"""최종 렌더: 승인된 작업만 `npx remotion render` (기획서 5.5).

GPU (사용자 결정 2026-11-14: 라데온도 지원)
- 화면 그리기: Windows 는 `--gl=angle` (D3D, 라데온·NVIDIA·인텔 모두 GPU 사용).
- 인코딩: NVIDIA 면 Remotion 번들 FFmpeg 의 NVENC(`--hardware-acceleration if-possible`).
  AMD·인텔이면 지금은 CPU(x264). 30~60초 세로 영상이라 몇십 초 수준. AMF 는 다음 단계.
"""

from __future__ import annotations

import os
import platform
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

from ..db import Store

RENDER_DIR = Path(__file__).resolve().parents[3] / "render"


@dataclass
class GpuInfo:
    vendor: str  # nvidia | amd | intel | none
    name: str


def detect_gpu() -> GpuInfo:
    names: list[str] = []
    try:
        if platform.system() == "Windows":
            out = subprocess.run(
                ["powershell", "-NoProfile", "-Command",
                 "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name"],
                capture_output=True, text=True, timeout=15, creationflags=0x0800_0000)
            names = [n.strip() for n in out.stdout.splitlines() if n.strip()]
        elif shutil.which("lspci"):
            out = subprocess.run(["lspci"], capture_output=True, text=True, timeout=15)
            names = [ln for ln in out.stdout.splitlines() if re.search(r"VGA|3D|Display", ln)]
    except (OSError, subprocess.SubprocessError):
        names = []
    joined = " ".join(names).lower()
    for vendor, keys in (("nvidia", ("nvidia", "geforce", "rtx")),
                         ("amd", ("amd", "radeon")), ("intel", ("intel",))):
        if any(k in joined for k in keys):
            return GpuInfo(vendor, next((n for n in names if any(k in n.lower() for k in keys)), ""))
    return GpuInfo("none", "")


def render_args(spec_path: Path, public_dir: Path, out_path: Path, encoder: str,
                browser: str | None = None, gl: str | None = None) -> list[str]:
    npx = shutil.which("npx") or "npx"
    args = [npx, "remotion", "render", "src/index.ts", "Story", str(out_path),
            f"--props={spec_path}", f"--public-dir={public_dir}", "--codec=h264",
            "--crf=18" if encoder != "nvenc" else "--video-bitrate=12M"]
    args.append("--hardware-acceleration=" + ("if-possible" if encoder == "nvenc" else "disable"))
    if gl is None and platform.system() == "Windows":
        gl = "angle"
    if gl:
        args.append(f"--gl={gl}")
    if browser:
        args.append(f"--browser-executable={browser}")
    return args


def choose_encoder(setting: str, gpu: GpuInfo) -> str:
    if setting in ("nvenc", "cpu"):
        return setting
    if setting == "amf":
        return "cpu"  # 아직 AMF 경로 없음 → CPU (README 참고)
    return "nvenc" if gpu.vendor == "nvidia" else "cpu"


def render_job(spec_path: Path, out_path: Path, encoder: str, browser: str | None = None,
               timeout: float = 1800) -> tuple[bool, str]:
    if not (RENDER_DIR / "node_modules").exists():
        return False, f"render 폴더에서 npm install 을 먼저 하세요 ({RENDER_DIR})"
    public = spec_path.parent / "public"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    flags = 0x0800_0000 if os.name == "nt" else 0
    try:
        proc = subprocess.run(render_args(spec_path, public, out_path, encoder, browser),
                              cwd=RENDER_DIR, capture_output=True, text=True, encoding="utf-8",
                              errors="replace", timeout=timeout, creationflags=flags)
    except (OSError, subprocess.TimeoutExpired) as err:
        return False, f"렌더 실행 실패: {err}"
    log = (proc.stdout or "") + (proc.stderr or "")
    if proc.returncode != 0 or not out_path.exists():
        return False, log[-1500:]
    return True, log[-1500:]


def render_approved(store: Store, out_dir: Path, setting: str = "auto",
                    browser: str | None = None, limit: int = 10) -> dict[str, int]:
    gpu = detect_gpu()
    encoder = choose_encoder(setting, gpu)
    rows = store.conn.execute(
        "SELECT id, spec_path FROM jobs WHERE status='approved' AND spec_path IS NOT NULL "
        "ORDER BY id LIMIT ?", (max(1, min(limit, 100)),)).fetchall()
    counts = {"rendered": 0, "failed": 0}
    for row in rows:
        out = out_dir / f"overshorts-{row['id']}.mp4"
        ok, log = render_job(Path(row["spec_path"]), out, encoder, browser)
        if ok:
            store.conn.execute("UPDATE jobs SET mp4_path=? WHERE id=?", (str(out), row["id"]))
            store.set_job_status(row["id"], "rendered", None)
            counts["rendered"] += 1
        else:
            store.set_job_status(row["id"], "approved", f"렌더: {log[-300:]}")
            counts["failed"] += 1
    return counts
