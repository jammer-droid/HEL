from auth.src.client import request_options


def test_override():
    assert request_options({"timeout_ms": 1200})["timeout_ms"] == 1200  # TIMEOUT_MS = 1200 in this test
