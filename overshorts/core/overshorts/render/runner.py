"""최종 렌더: 승인된 작업만 `npx remotion render` (기획서 5.5).

GPU (사용자 결정 2026-11-14: 라데온도 지원)
- 화면 그리기: Windows 는 `--gl=angle` (D3D, 라데온·NVIDIA·인텔 모두 GPU 사용).
- 인코딩: NVIDIA 면 Remotion 번들 FFmpeg 의 NVENC(`--hardware-acceleration if-possible`).
  AMD·인텔이면 지금은 CPU(x264). 30~60초 세로 영상이라 몇십 초 수준. AMF 는 다음 단계.
"""

from __future__ import annotations

import math
import os
import platform
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

from ..db import Store
from ..proc import decode_output
from .bundle import CACHE_NAME, ensure_bundle, make_serve_dir

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


REMOTION_CLI = RENDER_DIR / "node_modules" / "@remotion" / "cli" / "remotion-cli.js"


def remotion_command() -> list[str]:
    """Remotion CLI 를 node 로 바로 부른다. npx·node_modules/.bin 바로가기(.cmd)는 설치본에서
    빠지거나 깨질 수 있어 쓰지 않는다 (2026-11-15 설치본: 'remotion' 은 내부 또는 외부 명령이 아닙니다)."""
    node = shutil.which("node")
    if node and REMOTION_CLI.is_file():
        return [node, str(REMOTION_CLI)]
    return [shutil.which("npx") or "npx", "remotion"]


# --- 동시 탭 수 (최적화 측정 2026-11-21, 후보 1) ---
# Remotion 기본은 CPU 스레드의 절반. 4코어에서 4탭이면 카드형 −23%, 프레임형 −40% 였지만
# 탭마다 Chrome 과 배경 영상 프레임 캐시가 메모리를 먹는다 (2탭에서 5GB 까지 봄).
# 그래서 auto 는 메모리가 16GB(15GB 넘게 보임) 이상일 때만 늘리고, 모르거나 적으면 Remotion 기본 그대로 둔다.
# 사용자 PC(라데온 RX 9060)에서 실제 속도·메모리 확인 필요.
CONCURRENCY_CHOICES = ("auto", "1", "2", "3", "4", "6", "8", "12", "16")
AUTO_MIN_RAM_GB = 15  # 16GB PC 는 운영체제에 15.x GB 로 보인다
AUTO_MAX_TABS = 8
RAM_PER_TAB_GB = 2
RAM_RESERVED_GB = 8  # 운영체제·패널·Ollama 등 몫


def total_memory_gb() -> float | None:
    """전체 메모리(GB). 모르면 None."""
    try:
        if os.name == "nt":
            import ctypes

            class _MemStatus(ctypes.Structure):
                _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong),
                            ("ullTotalPhys", ctypes.c_ulonglong), ("ullAvailPhys", ctypes.c_ulonglong),
                            ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                            ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong),
                            ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]

            st = _MemStatus()
            st.dwLength = ctypes.sizeof(_MemStatus)
            if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st)):  # type: ignore[attr-defined]
                return st.ullTotalPhys / 2**30
            return None
        if hasattr(os, "sysconf") and "SC_PHYS_PAGES" in os.sysconf_names:
            pages, size = os.sysconf("SC_PHYS_PAGES"), os.sysconf("SC_PAGE_SIZE")
            if pages > 0 and size > 0:
                return pages * size / 2**30
    except (OSError, ValueError, AttributeError):
        return None
    return None


def auto_concurrency(cores: int, ram_gb: float | None) -> int:
    """Remotion 기본(스레드 절반)보다 줄이지는 않고, 메모리가 넉넉할 때만 늘린다."""
    cores = max(1, cores)
    half = max(1, cores // 2)
    if ram_gb is None or math.isnan(ram_gb) or ram_gb < AUTO_MIN_RAM_GB:
        return half
    by_ram = int((ram_gb - RAM_RESERVED_GB) // RAM_PER_TAB_GB)
    return max(half, min(cores, AUTO_MAX_TABS, by_ram))


def choose_concurrency(setting: object, cores: int | None = None, ram_gb: float | None = None) -> int:
    """설정 render.concurrency (auto 또는 숫자) → 동시 탭 수. 스레드 수를 넘지 않는다."""
    cores = max(1, cores if cores is not None else (os.cpu_count() or 1))
    text = str(setting if setting is not None else "auto").strip().lower()
    if text.isdigit():
        return max(1, min(int(text), cores))
    return auto_concurrency(cores, total_memory_gb() if ram_gb is None else ram_gb)


def render_args(spec_path: Path, public_dir: Path, out_path: Path, encoder: str,
                browser: str | None = None, gl: str | None = None, concurrency: int | None = None,
                serve_dir: Path | None = None) -> list[str]:
    """serve_dir (미리 만든 번들 + 작업 public) 가 있으면 번들 없이 바로 그린다."""
    entry = [str(serve_dir)] if serve_dir else ["src/index.ts"]
    public = [] if serve_dir else [f"--public-dir={public_dir}"]
    args = [*remotion_command(), "render", *entry, "Story", str(out_path),
            f"--props={spec_path}", *public, "--codec=h264",
            "--crf=18" if encoder != "nvenc" else "--video-bitrate=12M"]
    if concurrency:
        args.append(f"--concurrency={concurrency}")
    args.append("--hardware-acceleration=" + ("if-possible" if encoder == "nvenc" else "disable"))
    if gl is None and platform.system() == "Windows":
        gl = "angle"
    if gl:
        args.append(f"--gl={gl}")
    if browser:
        args.append(f"--browser-executable={browser}")
    if os.environ.get("OVERSHORTS_IGNORE_CERT"):  # 클라우드 시험용 (stage-export 와 같음)
        args.append("--ignore-certificate-errors")
    return args


def choose_encoder(setting: str, gpu: GpuInfo) -> str:
    if setting in ("nvenc", "cpu"):
        return setting
    if setting == "amf":
        return "cpu"  # 아직 AMF 경로 없음 → CPU (README 참고)
    return "nvenc" if gpu.vendor == "nvidia" else "cpu"


def render_job(spec_path: Path, out_path: Path, encoder: str, browser: str | None = None,
               timeout: float = 1800, concurrency: int | None = None,
               bundle_dir: Path | None = None) -> tuple[bool, str]:
    """bundle_dir: prepare_bundle() 결과. 없으면 예전처럼 Remotion 이 작업마다 번들한다."""
    if not REMOTION_CLI.is_file():
        return False, f"렌더 모듈이 없습니다. render 폴더에서 npm install 을 먼저 하세요 ({RENDER_DIR})"
    if not shutil.which("node"):
        return False, "Node.js 를 찾지 못했습니다 (설치본이면 OverShorts.exe 로 켜야 함께 든 Node 를 씁니다)"
    public = spec_path.parent / "public"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    flags = 0x0800_0000 if os.name == "nt" else 0
    serve = make_serve_dir(bundle_dir, public.resolve(), (spec_path.parent / "serve").resolve()) if bundle_dir else None
    args = render_args(spec_path, public, out_path, encoder, browser, concurrency=concurrency, serve_dir=serve)
    try:
        proc = subprocess.run(args, cwd=RENDER_DIR, capture_output=True, check=False, timeout=timeout,
                              creationflags=flags)
    except (OSError, subprocess.TimeoutExpired) as err:
        return False, f"렌더 실행 실패: {err}"
    finally:
        if serve:
            shutil.rmtree(serve, ignore_errors=True)
    head = f"[overshorts] 동시 탭 {concurrency or 'Remotion 기본'}, 번들 {'미리 만든 것' if serve else '작업마다 새로'}\n"
    log = head + decode_output(proc.stdout) + decode_output(proc.stderr)
    try:  # 전체 기록은 작업 폴더에 (화면에는 끝부분만)
        (spec_path.parent / "render.log").write_text(log, encoding="utf-8")
    except OSError:
        pass
    if proc.returncode != 0 or not out_path.exists():
        return False, log[-1500:]
    return True, log[-1500:]


def prepare_bundle(jobs_dir: Path) -> Path | None:
    """번들을 한 번 만들어 둔다 (렌더 여러 개가 같이 씀). 안 되면 None = 예전 방식."""
    if not REMOTION_CLI.is_file() or not shutil.which("node"):
        return None
    return ensure_bundle(RENDER_DIR, jobs_dir.resolve() / CACHE_NAME, remotion_command())


def render_approved(store: Store, out_dir: Path, setting: str = "auto",
                    browser: str | None = None, limit: int = 10,
                    concurrency: object = "auto") -> dict[str, int]:
    gpu = detect_gpu()
    encoder = choose_encoder(setting, gpu)
    tabs = choose_concurrency(concurrency)
    rows = store.conn.execute(
        "SELECT id, spec_path FROM jobs WHERE status='approved' AND spec_path IS NOT NULL "
        "ORDER BY id LIMIT ?", (max(1, min(limit, 100)),)).fetchall()
    counts = {"rendered": 0, "failed": 0}
    bundle_dir = prepare_bundle(Path(rows[0]["spec_path"]).parent.parent) if rows else None
    for row in rows:
        out = out_dir / f"overshorts-{row['id']}.mp4"
        ok, log = render_job(Path(row["spec_path"]), out, encoder, browser,
                             concurrency=tabs, bundle_dir=bundle_dir)
        if ok:
            store.conn.execute("UPDATE jobs SET mp4_path=? WHERE id=?", (str(out), row["id"]))
            store.set_job_status(row["id"], "rendered", None)
            counts["rendered"] += 1
        else:
            store.set_job_status(row["id"], "approved", f"렌더: {log[-300:]}")
            counts["failed"] += 1
    return counts
