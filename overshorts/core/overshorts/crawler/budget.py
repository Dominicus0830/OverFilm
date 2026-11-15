"""요청 예산, 랜덤 간격, 차단 감지와 쿨다운 (기획서 5.1).

차단을 피하는 핵심은 우회가 아니라 요청 수를 줄이는 것이다.
- 동시 요청 없음: 한 번에 한 요청만 (이 객체 하나가 모든 요청을 거친다).
- 요청마다 1.5~4초 랜덤 간격, 글 10개마다 20~40초 휴식.
- 시간당 60건, 하루 200건 상한. 넘으면 요청하지 않고 멈춘다.
- 403·429·캡차·무한 리디렉션 → 즉시 중단, 60분 쿨다운. 쿨다운 뒤 또 막히면 그날 수집 종료.
- 응답 시간이 평소의 3배 이상이면 간격을 2배로.
상태는 SQLite에 남겨서 프로그램을 다시 켜도 예산·쿨다운이 이어진다.
"""

from __future__ import annotations

import random
import statistics
import time
from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime, timedelta

from ..db import Store


@dataclass
class CrawlLimits:
    delay_sec: tuple[float, float] = (1.5, 4.0)
    rest_every: int = 10
    rest_sec: tuple[float, float] = (20.0, 40.0)
    hourly_limit: int = 60
    daily_limit: int = 200
    cooldown_min: int = 60
    slow_ratio: float = 3.0
    max_slow_factor: float = 8.0

    def warnings(self) -> list[str]:
        """패널에 띄울 '차단 위험' 경고 (UX 원칙 3)."""
        out = []
        if self.delay_sec[0] < 1.0:
            out.append("요청 간격이 1초 미만입니다: 차단 위험")
        if self.hourly_limit > 60:
            out.append("시간당 예산이 기본(60건)보다 큽니다: 차단 위험")
        if self.daily_limit > 200:
            out.append("하루 예산이 기본(200건)보다 큽니다: 차단 위험")
        return out


class CrawlStopped(Exception):
    """지금은 요청하면 안 된다 (예산 소진, 쿨다운, 그날 종료). 사유는 한국어."""

    def __init__(self, reason: str, retry_at: float | None = None):
        super().__init__(reason)
        self.reason = reason
        self.retry_at = retry_at


class CrawlBlocked(CrawlStopped):
    """방금 요청이 차단 신호를 받았다."""


class CrawlGuard:
    def __init__(
        self,
        store: Store,
        limits: CrawlLimits | None = None,
        clock: Callable[[], float] = time.time,
        sleep: Callable[[float], None] = time.sleep,
        rng: random.Random | None = None,
    ):
        self.store = store
        self.limits = limits or CrawlLimits()
        self.clock = clock
        self.sleep = sleep
        self.rng = rng or random.Random()
        self._requests_this_run = 0
        self._posts_since_rest = 0

    # --- 상태 ---
    def _today(self) -> str:
        return datetime.fromtimestamp(self.clock()).date().isoformat()

    def _midnight(self) -> float:
        now = datetime.fromtimestamp(self.clock())
        return datetime(now.year, now.month, now.day).timestamp()

    @property
    def requests_made(self) -> int:
        """이 실행에서 실제로 보낸 요청 수."""
        return self._requests_this_run

    def slow_factor(self) -> float:
        try:
            value = float(self.store.get_state("slow_factor", "1") or "1")
        except ValueError:
            value = 1.0
        return max(1.0, min(value, self.limits.max_slow_factor))

    def status(self) -> dict:
        """홈 화면 '크롤링 예산 게이지'용."""
        now = self.clock()
        cooldown_until = self._cooldown_until()
        return {
            "hour_used": self.store.count_requests_since(now - 3600),
            "hour_limit": self.limits.hourly_limit,
            "day_used": self.store.count_requests_since(self._midnight()),
            "day_limit": self.limits.daily_limit,
            "cooldown_until": cooldown_until if cooldown_until > now else None,
            "stopped_today": self.store.get_state("stopped_day") == self._today(),
            "slow_factor": self.slow_factor(),
        }

    def _cooldown_until(self) -> float:
        try:
            return float(self.store.get_state("cooldown_until", "0") or "0")
        except ValueError:
            return 0.0

    def check(self) -> None:
        """요청해도 되는지. 안 되면 CrawlStopped."""
        now = self.clock()
        if self.store.get_state("stopped_day") == self._today():
            tomorrow = datetime.fromtimestamp(self._midnight()) + timedelta(days=1)
            raise CrawlStopped("오늘 두 번 차단되어 수집을 끝냈습니다. 내일 다시 시도합니다.",
                               tomorrow.timestamp())
        until = self._cooldown_until()
        if until > now:
            mins = int((until - now) // 60) + 1
            raise CrawlStopped(f"차단 뒤 쿨다운 중입니다 ({mins}분 남음).", until)
        if self.store.count_requests_since(now - 3600) >= self.limits.hourly_limit:
            raise CrawlStopped(
                f"시간당 요청 예산 {self.limits.hourly_limit}건을 다 썼습니다.", now + 600
            )
        if self.store.count_requests_since(self._midnight()) >= self.limits.daily_limit:
            raise CrawlStopped(f"하루 요청 예산 {self.limits.daily_limit}건을 다 썼습니다.")

    # --- 간격 ---
    def wait_before_request(self) -> None:
        if self._requests_this_run == 0:
            return
        lo, hi = self.limits.delay_sec
        lo, hi = max(0.0, lo), max(lo, hi)
        self.sleep(self.rng.uniform(lo, hi) * self.slow_factor())

    def post_fetched(self) -> None:
        """본문 하나를 받은 뒤 부른다. 글 10개마다 긴 휴식."""
        self._posts_since_rest += 1
        if self.limits.rest_every > 0 and self._posts_since_rest >= self.limits.rest_every:
            self._posts_since_rest = 0
            lo, hi = self.limits.rest_sec
            self.sleep(self.rng.uniform(max(0.0, lo), max(lo, hi)))

    # --- 기록과 차단 대응 ---
    def record(self, url: str, status: int | None, elapsed: float | None, blocked: bool) -> None:
        now = self.clock()
        self._requests_this_run += 1
        outcome = "blocked" if blocked else ("ok" if status and 200 <= status < 400 else "error")
        if outcome == "ok" and elapsed is not None:
            self._check_slow(elapsed)
        self.store.log_request(now, url, status, elapsed, outcome)
        if blocked:
            self._on_block(now, status)

    def _check_slow(self, elapsed: float) -> None:
        recent = self.store.recent_elapsed()
        if len(recent) < 5:
            return
        median = statistics.median(recent)
        if median > 0 and elapsed >= self.limits.slow_ratio * median:
            factor = min(self.slow_factor() * 2, self.limits.max_slow_factor)
            self.store.set_state("slow_factor", str(factor))
        elif elapsed < 1.5 * median and self.slow_factor() > 1.0:
            self.store.set_state("slow_factor", str(max(1.0, self.slow_factor() / 2)))

    def _on_block(self, now: float, status: int | None) -> None:
        signal = f"HTTP {status}" if status else "캡차·리디렉션"
        today = self._today()
        if self.store.get_state("last_block_day") == today:
            self.store.set_state("stopped_day", today)
            raise CrawlBlocked(f"쿨다운 뒤 또 막혔습니다 ({signal}). 오늘 수집을 끝냅니다.")
        until = now + self.limits.cooldown_min * 60
        self.store.set_state("last_block_day", today)
        self.store.set_state("cooldown_until", str(until))
        raise CrawlBlocked(
            f"차단 신호({signal})를 받아 즉시 멈췄습니다. {self.limits.cooldown_min}분 쉽니다.",
            until,
        )
