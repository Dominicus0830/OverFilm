"""`overshorts` 명령 (MVP 동안 패널 대신 쓰는 입구).

    overshorts parse <링크>          링크 종류만 판단 (요청 안 함)
    overshorts ingest <링크> [옵션]   수집 + 1차 규칙 필터
    overshorts inspect                받은 그림 얼굴 흐림·선정성 검사 (모아서 한 번에)
    overshorts budget                 오늘 요청 예산·쿨다운 상태
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .config import crawl_limits, load_config, rule_config
from .crawler.budget import CrawlGuard
from .crawler.collector import Collector, GalleryOptions
from .crawler.fetcher import Fetcher
from .db import Store
from .urls import UrlError, parse_url


def _store(cfg: dict) -> tuple[Store, Path]:
    data = Path(str(cfg.get("data_dir") or "./data"))
    return Store(data / "overshorts.db"), data


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="overshorts", description="DC 갤러리 → 쇼츠 자동화")
    ap.add_argument("--config", default="config.yaml", help="설정 파일 (없으면 기본값)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("parse", help="링크 종류 판단")
    p.add_argument("url")

    i = sub.add_parser("ingest", help="수집 + 규칙 필터")
    i.add_argument("url")
    i.add_argument("--recommend-only", action="store_true", help="개념글만")
    i.add_argument("--min-upvotes", type=int, default=0)
    i.add_argument("--within-hours", type=float, default=24.0)
    i.add_argument("--max-posts", type=int, default=10)

    sub.add_parser("budget", help="요청 예산·쿨다운 상태")
    ins = sub.add_parser("inspect", help="받은 그림의 얼굴 흐림·선정성 검사")
    ins.add_argument("--no-download", action="store_true", help="모델을 받지 않음 (없으면 검사 못 함)")

    args = ap.parse_args(argv)
    cfg = load_config(args.config)

    if args.cmd == "parse":
        try:
            u = parse_url(args.url)
        except UrlError as err:
            print(f"알아볼 수 없는 링크: {err}", file=sys.stderr)
            return 2
        print(json.dumps({**u.__dict__, "label": u.label}, ensure_ascii=False))
        return 0

    store, data = _store(cfg)
    try:
        limits = crawl_limits(cfg)
        guard = CrawlGuard(store, limits)
        if args.cmd == "inspect":
            from .images import ImageInspector, inspect_pending

            f = cfg.get("filter") or {}
            inspector = ImageInspector(data / "models", download=not args.no_download,
                                       blur_faces=bool(f.get("blur_faces", True)),
                                       drop_nsfw=bool(f.get("drop_nsfw_images", True)))
            counts = inspect_pending(store, inspector, data / "images")
            print(f"글 {counts['posts']}개, 그림 {counts['images']}장 검사: 얼굴 흐림 "
                  f"{counts['faces']}장, 선정성으로 뺌 {counts['dropped']}장, "
                  f"검사 못 함 {counts['unchecked']}장 (장치: {inspector.provider or '없음'})")
            for e in inspector.load_errors:
                print(f"  경고: {e}", file=sys.stderr)
            return 0
        if args.cmd == "budget":
            print(json.dumps(guard.status(), ensure_ascii=False))
            for w in limits.warnings():
                print(f"경고: {w}", file=sys.stderr)
            return 0
        try:
            parse_url(args.url)
        except UrlError as err:
            print(f"알아볼 수 없는 링크: {err}", file=sys.stderr)
            return 2
        for w in limits.warnings():
            print(f"경고: {w}", file=sys.stderr)
        crawler = cfg.get("crawler") or {}
        fetcher = Fetcher(guard, proxy=crawler.get("proxy"))
        collector = Collector(
            store, fetcher, rule_config(cfg), cache_dir=data / "cache",
            max_comments=int(crawler.get("max_comments") or 50),
            image_dir=data / "images", max_images=int(crawler.get("max_images") or 5),
        )
        opts = GalleryOptions(
            recommend_only=args.recommend_only,
            min_upvotes=max(0, args.min_upvotes),
            within_hours=args.within_hours if args.within_hours > 0 else None,
            max_posts=max(1, min(args.max_posts, 200)),
        )
        report = collector.ingest(args.url, opts)
        print(report.summary())
        for no, reasons in report.blocked:
            print(f"  막힘 {no}: {'; '.join(reasons)}")
        for err in report.errors:
            print(f"  오류: {err}")
        return 1 if report.stopped and not report.saved else 0
    finally:
        store.close()


if __name__ == "__main__":
    raise SystemExit(main())
