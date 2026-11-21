"""렌더 준비(번들)를 한 번만 만들어 작업마다 다시 쓴다 (최적화 측정 2026-11-21, 후보 3).

예전에는 작업마다 `remotion render src/index.ts --public-dir=<작업 public>` 이라 매번 webpack 번들을
새로 만들고 public 폴더(배경 영상 포함)를 번들 안으로 복사했다 (30초 카드형 기준 약 12초).

지금은
1. `render/src`·package 파일 내용으로 키를 만들어 `<jobs>/.render-bundle/bundle-<키>/` 에 한 번 번들한다
   (public 은 빈 폴더로). 같은 키면 다시 쓴다. 템플릿이 바뀌면 키가 바뀌어 새로 만든다.
2. 작업마다 `<작업>/serve/` 에 번들 파일을 하드 링크로 놓고 (복사 없음, 같은 드라이브)
   `serve/public/` 에 작업 public 파일을 하드 링크로 놓는다. 하드 링크가 안 되면 복사.
3. `remotion render <serve> Story …` 로 그린다. 번들 내용과 렌더 옵션이 같아 화면은 프레임 단위로 같다
   (framemd5 비교, PR 설명 참고).

어느 단계든 실패하면 None 을 돌려주고, 부르는 쪽은 예전 방식(src/index.ts + --public-dir)으로 그린다.
"""

from __future__ import annotations

import hashlib
import logging
import os
import shutil
import subprocess
import uuid
from pathlib import Path

log = logging.getLogger(__name__)

CACHE_NAME = ".render-bundle"
DONE_MARK = ".overshorts-bundle-ok"
_KEY_FILES = ("package.json", "package-lock.json", "tsconfig.json")
MAX_SRC_FILES = 2000  # 키 계산 상한 (render/src 는 수십 개)


def bundle_key(render_dir: Path) -> str | None:
    """템플릿 소스·패키지 버전이 같으면 같은 키."""
    h = hashlib.sha256()
    src = render_dir / "src"
    try:
        files = sorted(p for p in src.rglob("*") if p.is_file())
    except OSError:
        return None
    if not files or len(files) > MAX_SRC_FILES:
        return None
    extra = [render_dir / n for n in _KEY_FILES if (render_dir / n).is_file()]
    for p in [*files, *extra]:
        try:
            data = p.read_bytes()
        except OSError:
            return None
        h.update(p.relative_to(render_dir).as_posix().encode("utf-8"))
        h.update(b"\0")
        h.update(hashlib.sha256(data).digest())
    return h.hexdigest()[:16]


def ensure_bundle(render_dir: Path, cache_root: Path, cli: list[str], timeout: float = 600) -> Path | None:
    """번들 폴더 (이미 있으면 그대로). 실패하면 None."""
    key = bundle_key(render_dir)
    if key is None:
        return None
    target = cache_root / f"bundle-{key}"
    if (target / DONE_MARK).is_file() and (target / "index.html").is_file():
        return target
    tmp = cache_root / f"tmp-{key}-{uuid.uuid4().hex[:8]}"
    try:
        (tmp / "_empty_public").mkdir(parents=True, exist_ok=True)
        out = tmp / "bundle"
        proc = subprocess.run(
            [*cli, "bundle", "src/index.ts", f"--out-dir={out}", f"--public-dir={tmp / '_empty_public'}"],
            cwd=render_dir, capture_output=True, check=False, timeout=timeout,
            creationflags=0x0800_0000 if os.name == "nt" else 0)
        if proc.returncode != 0 or not (out / "index.html").is_file():
            log.warning("렌더 번들 만들기 실패 (예전 방식으로 그림): %s",
                        (proc.stderr or proc.stdout or b"")[-500:].decode("utf-8", "replace"))
            return None
        (out / DONE_MARK).write_text(key, encoding="utf-8")
        try:
            out.rename(target)
        except OSError:
            # 다른 렌더가 먼저 만들었으면 그것을 쓴다
            if not (target / DONE_MARK).is_file():
                return None
        _drop_old(cache_root, keep=target.name)
        return target
    except (OSError, subprocess.SubprocessError) as err:
        log.warning("렌더 번들 만들기 실패 (예전 방식으로 그림): %s", err)
        return None
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def _drop_old(cache_root: Path, keep: str) -> None:
    """템플릿이 바뀌어 안 쓰는 옛 번들 (하나에 약 40MB) 지우기."""
    try:
        for p in cache_root.iterdir():
            if p.is_dir() and p.name.startswith("bundle-") and p.name != keep:
                shutil.rmtree(p, ignore_errors=True)
    except OSError:
        pass


def _link_or_copy(src: Path, dst: Path) -> None:
    try:
        os.link(src, dst)
    except OSError:
        shutil.copy2(src, dst)


def _link_tree(src: Path, dst: Path, skip: str | None = None) -> None:
    dst.mkdir(parents=True, exist_ok=True)
    for p in src.iterdir():
        if p.name == skip or p.name == DONE_MARK:
            continue
        if p.is_dir():
            _link_tree(p, dst / p.name)
        elif p.is_file():
            _link_or_copy(p, dst / p.name)


def make_serve_dir(bundle: Path, public: Path, dest: Path) -> Path | None:
    """작업 한 개용 serve 폴더 = 번들 + 작업 public (하드 링크). 실패하면 None."""
    try:
        shutil.rmtree(dest, ignore_errors=True)
        _link_tree(bundle, dest, skip="public")
        if public.is_dir():
            _link_tree(public, dest / "public")
        else:
            (dest / "public").mkdir(parents=True, exist_ok=True)
        return dest
    except OSError as err:
        log.warning("렌더 serve 폴더 만들기 실패 (예전 방식으로 그림): %s", err)
        shutil.rmtree(dest, ignore_errors=True)
        return None
