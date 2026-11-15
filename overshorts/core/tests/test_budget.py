import pytest

from overshorts.crawler.budget import CrawlBlocked, CrawlGuard, CrawlLimits, CrawlStopped
from overshorts.db import Store


class Clock:
    def __init__(self, t=1_800_000_000.0):
        self.t = t

    def __call__(self):
        return self.t


def make(limits=None):
    clock = Clock()
    sleeps = []
    store = Store(":memory:")
    guard = CrawlGuard(store, limits or CrawlLimits(), clock=clock, sleep=sleeps.append)
    return guard, clock, sleeps


def test_hourly_and_daily_budget():
    guard, clock, _ = make(CrawlLimits(hourly_limit=3, daily_limit=5))
    for _ in range(3):
        guard.check()
        guard.record("u", 200, 0.1, False)
    with pytest.raises(CrawlStopped, match="시간당"):
        guard.check()
    clock.t += 3601
    guard.check()


def test_random_delay_and_rest():
    guard, _, sleeps = make()
    guard.wait_before_request()
    assert sleeps == []  # 첫 요청은 바로
    guard.record("u", 200, 0.1, False)
    guard.wait_before_request()
    assert 1.5 <= sleeps[-1] <= 4.0
    for _ in range(10):
        guard.post_fetched()
    assert 20.0 <= sleeps[-1] <= 40.0


def test_block_cooldown_then_stop_for_day():
    guard, clock, _ = make()
    with pytest.raises(CrawlBlocked, match="60분"):
        guard.record("u", 429, 0.1, True)
    with pytest.raises(CrawlStopped, match="쿨다운"):
        guard.check()
    clock.t += 61 * 60
    guard.check()
    with pytest.raises(CrawlBlocked, match="오늘"):
        guard.record("u", 403, 0.1, True)
    with pytest.raises(CrawlStopped, match="오늘"):
        guard.check()


def test_slow_response_doubles_delay():
    guard, _, sleeps = make()
    for _ in range(6):
        guard.record("u", 200, 0.2, False)
    guard.record("u", 200, 1.0, False)
    assert guard.slow_factor() == 2.0
    guard.wait_before_request()
    assert 3.0 <= sleeps[-1] <= 8.0


def test_warnings():
    assert CrawlLimits(delay_sec=(0.5, 1.0)).warnings()
    assert not CrawlLimits().warnings()
