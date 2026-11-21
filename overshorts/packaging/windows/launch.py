"""OverShorts 실행기: OverShorts.exe 가 함께 든 Python 으로 이 파일을 돌린다.

사용자 데이터(설정, 작업, 스타일)는 %LOCALAPPDATA%\\OverShorts 에 두고, 함께 든 Node·FFmpeg 를
PATH 앞에 넣는다. 인자가 없으면(더블클릭) OverShorts 창으로 연다 (`serve --window`): 콘솔 창은 없고,
출력은 logs\\overshorts.log 에 남는다 (패널 '작업 큐 ▸ 서버 기록'에서 본다). 창을 닫으면 꺼진다.
"""
from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

APP = Path(__file__).resolve().parent
ROOT = APP.parent


def data_home() -> Path:
    env = os.environ.get("OVERSHORTS_HOME")
    if env:
        return Path(env)
    base = os.environ.get("LOCALAPPDATA") or str(Path.home())
    return Path(base) / "OverShorts"


def prepare(data: Path) -> Path:
    data.mkdir(parents=True, exist_ok=True)
    style = data / "style"
    if not style.exists():
        shutil.copytree(APP / "style", style)
    (data / "assets" / "bg").mkdir(parents=True, exist_ok=True)
    cfg = data / "config.yaml"
    if not cfg.exists():
        cfg.write_text(
            "# OverShorts 설정. 항목 설명은 설치 폴더 app\\config.example.yaml 참고.\n"
            "style:\n  dir: ./style\n"
            "data_dir: ./data\n",
            encoding="utf-8",
        )
    return cfg


def log_to_file(data: Path) -> None:
    """창 모드는 콘솔이 없으니 출력(오류 포함)을 로그 파일로 보낸다. 2MB 넘으면 하나 남기고 새로."""
    logs = data / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    path = logs / "overshorts.log"
    try:
        if path.is_file() and path.stat().st_size > 2_000_000:
            os.replace(path, logs / "overshorts.1.log")
    except OSError:
        pass  # 다른 OverShorts 가 쓰는 중이면 그대로 이어 쓴다
    f = open(path, "a", encoding="utf-8", errors="replace", buffering=1)  # noqa: SIM115 - 끝날 때까지 연다
    for fd in (1, 2):  # 하위 프로그램(ffmpeg·node)의 출력도 같은 파일로
        try:
            os.dup2(f.fileno(), fd)
        except OSError:
            pass
    sys.stdout = sys.stderr = f
    os.environ["OVERSHORTS_LOG"] = str(path)
    import datetime

    print(f"\n===== OverShorts 시작 {datetime.datetime.now().isoformat(timespec='seconds')} =====")


def main() -> int:
    data = data_home()
    args = sys.argv[1:] or ["serve", "--window"]
    if "--window" in args:  # 첫 준비(스타일 복사)에서 난 오류도 로그에 남게 먼저
        data.mkdir(parents=True, exist_ok=True)
        log_to_file(data)
    cfg = prepare(data)
    tools = [str(ROOT / "ffmpeg"), str(ROOT / "node")]
    os.environ["PATH"] = os.pathsep.join(tools + [os.environ.get("PATH", "")])
    os.environ.setdefault("PYTHONUTF8", "1")
    os.chdir(data)
    if "--config" not in args:
        args = ["--config", str(cfg), *args]
    print(f"OverShorts 데이터 폴더: {data}")
    from overshorts.cli import main as cli_main

    return cli_main(args)


if __name__ == "__main__":
    sys.exit(main())
